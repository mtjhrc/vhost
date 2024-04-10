// Copyright (C) 2024 Red Hat, Inc.
// SPDX-License-Identifier: Apache-2.0

use std::os::fd::{AsRawFd, RawFd};
use std::os::unix::net::UnixStream;
use std::sync::{Arc, Mutex, MutexGuard};
use std::{io, mem, slice};

use vm_memory::ByteValued;

use crate::vhost_user;
use crate::vhost_user::connection::Endpoint;
use crate::vhost_user::gpu_message::*;
use crate::vhost_user::message::{VhostUserMsgValidator, VhostUserU64};
use crate::vhost_user::Error;

struct BackendInternal {
    sock: Endpoint<VhostUserGpuMsgHeader<GpuBackendReq>>,
    // whether the endpoint has encountered any failure
    error: Option<i32>,
}

impl BackendInternal {
    fn check_state(&self) -> vhost_user::Result<u64> {
        match self.error {
            Some(e) => Err(Error::SocketBroken(io::Error::from_raw_os_error(e))),
            None => Ok(0),
        }
    }

    fn send_header<V: ByteValued + Sized + Default + VhostUserMsgValidator>(
        &mut self,
        request: GpuBackendReq,
        fds: Option<&[RawFd]>,
    ) -> vhost_user::Result<V> {
        self.check_state()?;

        let hdr = VhostUserGpuMsgHeader::new(request, 0, 0);
        self.sock.send_header(&hdr, fds)?;

        self.wait_for_ack(&hdr)
    }

    fn send_message_no_reply<T: ByteValued>(
        &mut self,
        request: GpuBackendReq,
        body: &T,
        fds: Option<&[RawFd]>,
    ) -> vhost_user::Result<()> {
        self.check_state()?;

        let len = mem::size_of::<T>();
        let hdr = VhostUserGpuMsgHeader::new(request, 0, len as u32);
        self.sock.send_message(&hdr, body, fds)?;
        Ok(())
    }

    fn send_message_with_payload_no_reply<T: ByteValued>(
        &mut self,
        request: GpuBackendReq,
        body: &T,
        data: &[u8],
        fds: Option<&[RawFd]>,
    ) -> vhost_user::Result<()> {
        self.check_state()?;

        let len = mem::size_of::<T>() + data.len();
        let hdr = VhostUserGpuMsgHeader::new(request, 0, len as u32);
        self.sock.send_message_with_payload(&hdr, body, data, fds)?;
        Ok(())
    }

    fn send_message<T: ByteValued, V: ByteValued + Sized + Default + VhostUserMsgValidator>(
        &mut self,
        request: GpuBackendReq,
        body: &T,
        fds: Option<&[RawFd]>,
    ) -> vhost_user::Result<V> {
        self.check_state()?;

        let len = mem::size_of::<T>();
        let hdr = VhostUserGpuMsgHeader::new(request, 0, len as u32);
        self.sock.send_message(&hdr, body, fds)?;

        self.wait_for_ack(&hdr)
    }

    fn wait_for_ack<V: ByteValued + Sized + Default + VhostUserMsgValidator>(
        &mut self,
        hdr: &VhostUserGpuMsgHeader<GpuBackendReq>,
    ) -> vhost_user::Result<V> {
        self.check_state()?;
        let (reply, body, rfds) = self.sock.recv_body::<V>()?;
        if !reply.is_reply_for(hdr) || rfds.is_some() || !body.is_valid() {
            return Err(Error::InvalidMessage);
        }
        Ok(body)
    }
}

/// Proxy for sending messages from the backend to the fronted
/// over the socket obtained from VHOST_USER_GPU_SET_SOCKET.
/// The protocol is documented here: https://www.qemu.org/docs/master/interop/vhost-user-gpu.html
#[derive(Clone)]
pub struct GpuBackend {
    // underlying Unix domain socket for communication
    node: Arc<Mutex<BackendInternal>>,
}

impl GpuBackend {
    fn new(ep: Endpoint<VhostUserGpuMsgHeader<GpuBackendReq>>) -> Self {
        Self {
            node: Arc::new(Mutex::new(BackendInternal {
                sock: ep,
                error: None,
            })),
        }
    }

    fn node(&self) -> MutexGuard<BackendInternal> {
        self.node.lock().unwrap()
    }

    fn send_header<V: ByteValued + Sized + Default + VhostUserMsgValidator>(
        &self,
        request: GpuBackendReq,
        fds: Option<&[RawFd]>,
    ) -> io::Result<V> {
        self.node()
            .send_header(request, fds)
            .map_err(|e| io::Error::new(io::ErrorKind::Other, format!("{}", e)))
    }

    fn send_message<T: ByteValued, V: ByteValued + Sized + Default + VhostUserMsgValidator>(
        &self,
        request: GpuBackendReq,
        body: &T,
        fds: Option<&[RawFd]>,
    ) -> io::Result<V> {
        self.node()
            .send_message(request, body, fds)
            .map_err(|e| io::Error::new(io::ErrorKind::Other, format!("{}", e)))
    }

    fn send_message_no_reply<T: ByteValued>(
        &self,
        request: GpuBackendReq,
        body: &T,
        fds: Option<&[RawFd]>,
    ) -> io::Result<()> {
        self.node()
            .send_message_no_reply(request, body, fds)
            .map_err(|e| io::Error::new(io::ErrorKind::Other, format!("{}", e)))
    }

    fn send_message_with_payload_no_reply<T: ByteValued>(
        &self,
        request: GpuBackendReq,
        body: &T,
        data: &[u8],
        fds: Option<&[RawFd]>,
    ) -> io::Result<()> {
        self.node()
            .send_message_with_payload_no_reply(request, body, data, fds)
            .map_err(|e| io::Error::new(io::ErrorKind::Other, format!("{}", e)))
    }

    /// Send the VHOST_USER_GPU_GET_PROTOCOL_FEATURES message to the frontend and wait for a reply.
    /// Get the supported protocol features bitmask.
    pub fn get_protocol_features(&self) -> io::Result<VhostUserU64> {
        self.send_header(GpuBackendReq::GET_PROTOCOL_FEATURES, None)
    }

    /// Send the VHOST_USER_GPU_SET_PROTOCOL_FEATURES message to the frontend. Doesn't wait for
    /// a reply.
    /// Enable protocol features using a bitmask.
    pub fn set_protocol_features(&self, msg: &VhostUserU64) -> io::Result<()> {
        self.send_message_no_reply(GpuBackendReq::SET_PROTOCOL_FEATURES, msg, None)
    }

    /// Send the VHOST_USER_GPU_GET_DISPLAY_INFO message to the frontend and wait for a reply.
    /// Get the preferred display configuration.
    pub fn get_display_info(&self) -> io::Result<VirtioGpuRespDisplayInfo> {
        self.send_header(GpuBackendReq::GET_DISPLAY_INFO, None)
    }

    /// Send the VHOST_USER_GPU_GET_EDID message to the frontend and wait for a reply.
    /// Retrieve the EDID data for a given scanout.
    /// This message requires the VHOST_USER_GPU_PROTOCOL_F_EDID protocol feature to be supported.
    pub fn get_edid(&self, get_edid: &VhostUserGpuEdidRequest) -> io::Result<VirtioGpuRespGetEdid> {
        self.send_message(GpuBackendReq::GET_EDID, get_edid, None)
    }

    /// Send the VHOST_USER_GPU_SCANOUT message to the frontend. Doesn't wait for a reply.
    /// Set the scanout resolution. To disable a scanout, the dimensions width/height are set to 0.
    pub fn set_scanout(&self, scanout: &VhostUserGpuScanout) -> io::Result<()> {
        self.send_message_no_reply(GpuBackendReq::SCANOUT, scanout, None)
    }

    /// Sends the VHOST_USER_GPU_UPDATE  message to the frontend. Doesn't wait for a reply.
    /// Updates the scanout content. The data payload contains the graphical bits.
    /// The display should be flushed and presented.
    pub fn update_scanout(&self, update: &VhostUserGpuUpdate, data: &[u8]) -> io::Result<()> {
        self.send_message_with_payload_no_reply(GpuBackendReq::UPDATE, update, data, None)
    }

    /// Send the VHOST_USER_GPU_DMABUF_SCANOUT  message to the frontend. Doesn't wait for a reply.
    /// Set the scanout resolution/configuration, and share a DMABUF file descriptor for the scanout
    /// content, which is passed as ancillary data. To disable a scanout, the dimensions
    /// width/height are set to 0, there is no file descriptor passed.
    pub fn set_dmabuf_scanout(
        &self,
        scanout: &VhostUserGpuDMABUFScanout,
        fd: Option<&impl AsRawFd>,
    ) -> io::Result<()> {
        let fd = fd.map(AsRawFd::as_raw_fd);
        let fd = fd.as_ref().map(slice::from_ref);
        self.send_message_no_reply(GpuBackendReq::DMABUF_SCANOUT, scanout, fd)
    }

    /// Send the VHOST_USER_GPU_DMABUF_UPDATE  message to the frontend. Doesn't wait for a reply.
    /// The display should be flushed and presented according to updated region
    /// from VhostUserGpuUpdate.
    pub fn update_dmabuf_scanout(&self, update: &VhostUserGpuUpdate) -> io::Result<()> {
        self.send_message_no_reply(GpuBackendReq::DMABUF_UPDATE, update, None)
    }

    /// Send the VHOST_USER_GPU_CURSOR_POS  message to the frontend. Doesn't wait for a reply.
    /// Set/show the cursor position.
    pub fn cursor_pos(&self, cursor_pos: &VhostUserGpuCursorPos) -> io::Result<()> {
        self.send_message_no_reply(GpuBackendReq::CURSOR_POS, cursor_pos, None)
    }

    /// Send the VHOST_USER_GPU_CURSOR_POS_HIDE  message to the frontend. Doesn't wait for a reply.
    /// Set/hide the cursor.
    pub fn cursor_pos_hide(&self, cursor_pos: &VhostUserGpuCursorPos) -> io::Result<()> {
        self.send_message_no_reply(GpuBackendReq::CURSOR_POS_HIDE, cursor_pos, None)
    }

    /// Send the VHOST_USER_GPU_CURSOR_UPDATE  message to the frontend. Doesn't wait for a reply.
    /// Update the cursor shape and location.
    /// `data` represents a 64*64 cursor image (PIXMAN_x8r8g8b8 format).
    pub fn cursor_update(
        &self,
        cursor_update: &VhostUserGpuCursorUpdate,
        data: &[u8; 4 * 64 * 64],
    ) -> io::Result<()> {
        self.send_message_with_payload_no_reply(
            GpuBackendReq::CURSOR_UPDATE,
            cursor_update,
            data,
            None,
        )
    }

    /// Create a new instance from a `UnixStream` object.
    pub fn from_stream(sock: UnixStream) -> Self {
        Self::new(Endpoint::<VhostUserGpuMsgHeader<GpuBackendReq>>::from_stream(sock))
    }

    /// Mark endpoint as failed with specified error code.
    pub fn set_failed(&self, error: i32) {
        self.node().error = Some(error);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_gpu_backend_req_set_failed() {
        let (p1, _p2) = UnixStream::pair().unwrap();
        let backend = GpuBackend::from_stream(p1);
        assert!(backend.node().error.is_none());
        backend.set_failed(libc::EAGAIN);
        assert_eq!(backend.node().error, Some(libc::EAGAIN));
    }
}
