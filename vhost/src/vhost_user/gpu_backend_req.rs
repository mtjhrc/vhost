use crate::vhost_user;
use crate::vhost_user::connection::Endpoint;
use crate::vhost_user::gpu_message::{
    GpuBackendReq, VhostUserGpuMsgHeader, VirtioGpuRespDisplayInfo,
};
use crate::vhost_user::message::VhostUserMsgValidator;
use crate::vhost_user::Error;
use std::os::fd::{AsRawFd, RawFd};
use std::os::unix::net::UnixStream;
use std::sync::{Arc, Mutex, MutexGuard};
use std::{io, mem};
use vm_memory::ByteValued;

use super::gpu_message::{
    VhostUserGpuDMABUFScanout, VhostUserGpuEdidRequest, VhostUserGpuScanout, VhostUserGpuUpdate,
    VirtioGpuRespGetEdid,
};

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

        //investigate what len should be
        let len = mem::size_of::<T>();
        let hdr = VhostUserGpuMsgHeader::new(request, 0, len as u32);
        self.sock.send_message(&hdr, body, fds)?;
        Ok(())
    }

    fn send_message_with_payload<T: ByteValued>(
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

        //investigate what len should be
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

#[allow(missing_docs)]
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
        &mut self,
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

    fn send_message_with_payload<T: ByteValued>(
        &self,
        request: GpuBackendReq,
        body: &T,
        data: &[u8],
        fds: Option<&[RawFd]>,
    ) -> io::Result<()> {
        self.node()
            .send_message_with_payload(request, body, data, fds)
            .map_err(|e| io::Error::new(io::ErrorKind::Other, format!("{}", e)))
    }

    /// Forward 2d gpu command get_display_info requests to the frontend.
    pub fn get_display_info(&mut self) -> io::Result<VirtioGpuRespDisplayInfo> {
        self.send_header(GpuBackendReq::GET_DISPLAY_INFO, None)
    }

    /// Forward 2d gpu command get_edid requests to the frontend.
    pub fn get_edid(
        &mut self,
        get_edid: &VhostUserGpuEdidRequest,
    ) -> io::Result<VirtioGpuRespGetEdid> {
        self.send_message(GpuBackendReq::GET_EDID, &get_edid.scanout_id, None)
    }

    /// Forward 2d gpu command SetScanout requests to the frontend.
    pub fn set_scanout(&mut self, scanout: &VhostUserGpuScanout) -> io::Result<()> {
        self.send_message_no_reply(GpuBackendReq::SCANOUT, scanout, None)
            .map_err(|e| io::Error::new(io::ErrorKind::Other, format!("{}", e)))
    }

    /// Forward 2d gpu command for SetScanout and sharing DMABUF fd requests to the frontend.
    pub fn set_dmabuf_scanout(
        &mut self,
        scanout: &VhostUserGpuDMABUFScanout,
        fd: &dyn AsRawFd,
    ) -> io::Result<()> {
        self.send_message_no_reply(
            GpuBackendReq::DMABUF_SCANOUT,
            scanout,
            Some(&[fd.as_raw_fd()]),
        )
        .map_err(|e| io::Error::new(io::ErrorKind::Other, format!("{}", e)))
    }

    /// Forward 2d gpu command Update Scanout requests to the frontend.
    pub fn update_scanout(&mut self, update: &VhostUserGpuUpdate, data: &[u8]) -> io::Result<()> {
        self.send_message_with_payload(GpuBackendReq::UPDATE, update, data, None)
            .map_err(|e| io::Error::new(io::ErrorKind::Other, format!("{}", e)))
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
