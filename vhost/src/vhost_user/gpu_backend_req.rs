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

    fn send_message_oversized_no_reply<T: ByteValued>(
        &mut self,
        request: GpuBackendReq,
        body: &T,
        fds: Option<&[RawFd]>,
    ) -> vhost_user::Result<()> {
        self.check_state()?;

        let len = mem::size_of::<T>();
        let hdr = VhostUserGpuMsgHeader::new(request, 0, len as u32);
        self.sock.send_message_oversized(&hdr, body, fds)?;
        Ok(())
    }

    fn send_message_oversized_with_payload_no_reply<T: ByteValued>(
        &mut self,
        request: GpuBackendReq,
        body: &T,
        data: &[u8],
        fds: Option<&[RawFd]>,
    ) -> vhost_user::Result<()> {
        self.check_state()?;

        let len = mem::size_of::<T>() + data.len();
        let hdr = VhostUserGpuMsgHeader::new(request, 0, len as u32);
        self.sock
            .send_message_oversized_with_payload(&hdr, body, data, fds)?;
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

    fn send_message_oversized_no_reply<T: ByteValued>(
        &self,
        request: GpuBackendReq,
        body: &T,
        fds: Option<&[RawFd]>,
    ) -> io::Result<()> {
        self.node()
            .send_message_oversized_no_reply(request, body, fds)
            .map_err(|e| io::Error::new(io::ErrorKind::Other, format!("{}", e)))
    }

    fn send_message_oversized_with_payload_no_reply<T: ByteValued>(
        &self,
        request: GpuBackendReq,
        body: &T,
        data: &[u8],
        fds: Option<&[RawFd]>,
    ) -> io::Result<()> {
        self.node()
            .send_message_oversized_with_payload_no_reply(request, body, data, fds)
            .map_err(|e| io::Error::new(io::ErrorKind::Other, format!("{}", e)))
    }

    /// Send the VHOST_USER_GPU_GET_PROTOCOL_FEATURES message to the frontend and wait for a reply.
    /// Get the supported protocol features bitmask.
    pub fn get_protocol_features(&self) -> io::Result<VhostUserU64> {
        self.send_header(GpuBackendReq::GET_PROTOCOL_FEATURES, None)
    }

    /// Send the VHOST_USER_GPU_GET_DISPLAY_INFO message to the frontend and wait for a reply.
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
        self.send_message_oversized_with_payload_no_reply(GpuBackendReq::UPDATE, update, data, None)
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

    /// Send the VHOST_USER_GPU_DMABUF_SCANOUT2  message to the frontend. Doesn't wait for a reply.
    /// Same as `set_dmabuf_scanout` (VHOST_USER_GPU_DMABUF_SCANOUT), but also sends the dmabuf
    /// modifiers appended to the message, which were not provided in the other message. This
    /// message requires the VhostUserGpuProtocolFeatures::DMABUF2
    /// (VHOST_USER_GPU_PROTOCOL_F_DMABUF2) protocol feature to be supported.
    pub fn set_dmabuf_scanout2(
        &self,
        scanout: &VhostUserGpuDMABUFScanout2,
        fd: Option<&impl AsRawFd>,
    ) -> io::Result<()> {
        let fd = fd.map(AsRawFd::as_raw_fd);
        let fd = fd.as_ref().map(slice::from_ref);
        self.send_message_no_reply(GpuBackendReq::DMABUF_SCANOUT2, scanout, fd)
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
    pub fn cursor_update(&self, cursor_update: &VhostUserGpuCursorUpdate) -> io::Result<()> {
        self.send_message_oversized_no_reply(GpuBackendReq::CURSOR_UPDATE, cursor_update, None)
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
    use crate::vhost_user::message::VhostUserEmpty;
    use libc::STDOUT_FILENO;
    use std::mem::{size_of, size_of_val};
    use std::os::fd::RawFd;
    use std::thread;

    #[derive(Default, Copy, Clone, Debug)]
    struct MockUnreachableResponse;

    // SAFETY: Safe because type is zero size.
    unsafe impl ByteValued for MockUnreachableResponse {}

    impl VhostUserMsgValidator for MockUnreachableResponse {
        fn is_valid(&self) -> bool {
            panic!("UnreachableResponse should not have been constructed and validated!");
        }
    }

    #[derive(Copy, Default, Clone, Debug, PartialEq)]
    struct MockData(u32);

    // SAFETY: Safe because type is POD.
    unsafe impl ByteValued for MockData {}

    impl VhostUserMsgValidator for MockData {}

    const REQUEST: GpuBackendReq = GpuBackendReq::GET_PROTOCOL_FEATURES;
    const VALID_REQUEST: MockData = MockData(123);
    const VALID_RESPONSE: MockData = MockData(456);

    fn reply_with_valid_response(
        frontend: &mut Endpoint<VhostUserGpuMsgHeader<GpuBackendReq>>,
        hdr: &VhostUserGpuMsgHeader<GpuBackendReq>,
    ) {
        let response_hdr = VhostUserGpuMsgHeader::new(
            hdr.get_code().unwrap(),
            VhostUserGpuHeaderFlag::REPLY.bits(),
            size_of::<MockData>() as u32,
        );

        frontend
            .send_message(&response_hdr, &VALID_RESPONSE, None)
            .unwrap();
    }

    fn frontend_backend_pair() -> (Endpoint<VhostUserGpuMsgHeader<GpuBackendReq>>, GpuBackend) {
        let (backend, frontend) = UnixStream::pair().unwrap();
        let backend = GpuBackend::from_stream(backend);
        let frontend = Endpoint::from_stream(frontend);

        (frontend, backend)
    }

    #[test]
    fn test_gpu_backend_req_set_failed() {
        let (p1, _p2) = UnixStream::pair().unwrap();
        let backend = GpuBackend::from_stream(p1);
        assert!(backend.node().error.is_none());
        backend.set_failed(libc::EAGAIN);
        assert_eq!(backend.node().error, Some(libc::EAGAIN));
    }

    #[test]
    fn test_gpu_backend_cannot_send_when_failed() {
        let (_frontend, backend) = frontend_backend_pair();
        backend.set_failed(libc::EAGAIN);

        backend
            .send_header::<MockData>(GpuBackendReq::GET_PROTOCOL_FEATURES, None)
            .unwrap_err();

        backend
            .send_message::<VhostUserEmpty, MockData>(
                GpuBackendReq::GET_PROTOCOL_FEATURES,
                &VhostUserEmpty,
                None,
            )
            .unwrap_err();

        backend
            .send_message_no_reply::<VhostUserEmpty>(
                GpuBackendReq::GET_PROTOCOL_FEATURES,
                &VhostUserEmpty,
                None,
            )
            .unwrap_err();

        backend
            .send_message_oversized_no_reply::<VhostUserEmpty>(
                GpuBackendReq::GET_PROTOCOL_FEATURES,
                &VhostUserEmpty,
                None,
            )
            .unwrap_err();

        backend
            .send_message_oversized_with_payload_no_reply::<VhostUserEmpty>(
                GpuBackendReq::GET_PROTOCOL_FEATURES,
                &VhostUserEmpty,
                &[],
                None,
            )
            .unwrap_err();
    }

    #[test]
    fn test_gpu_backend_send_header() {
        let (mut frontend, backend) = frontend_backend_pair();

        let sender_thread = thread::spawn(move || {
            let response: MockData = backend.send_header(REQUEST, None).unwrap();
            assert_eq!(response, VALID_RESPONSE);
        });

        let (hdr, fds) = frontend.recv_header().unwrap();
        assert!(fds.is_none());

        assert_eq!(hdr, VhostUserGpuMsgHeader::new(REQUEST, 0, 0));
        reply_with_valid_response(&mut frontend, &hdr);
        sender_thread.join().expect("Sender failed");
    }

    #[test]
    fn test_gpu_backend_send_message() {
        let (mut frontend, backend) = frontend_backend_pair();

        let sender_thread = thread::spawn(move || {
            let response: MockData = backend.send_message(REQUEST, &VALID_REQUEST, None).unwrap();
            assert_eq!(response, VALID_RESPONSE);
        });

        let (hdr, body, fds) = frontend.recv_body::<MockData>().unwrap();
        let expected_hdr =
            VhostUserGpuMsgHeader::new(REQUEST, 0, size_of_val(&VALID_REQUEST) as u32);
        assert_eq!(hdr, expected_hdr);
        assert_eq!(body, VALID_REQUEST);
        assert!(fds.is_none());

        reply_with_valid_response(&mut frontend, &hdr);
        sender_thread.join().expect("Sender failed");
    }

    #[test]
    fn test_gpu_backend_send_message_no_reply_with_fd() {
        let (mut frontend, backend) = frontend_backend_pair();
        let expected_hdr =
            VhostUserGpuMsgHeader::new(REQUEST, 0, size_of_val(&VALID_REQUEST) as u32);

        let requested_fds: &[RawFd] = &[STDOUT_FILENO];

        let sender_thread = thread::spawn(move || {
            backend
                .send_message_no_reply(REQUEST, &VALID_REQUEST, Some(requested_fds))
                .unwrap();
        });

        let (hdr, body, fds) = frontend.recv_body::<MockData>().unwrap();
        assert_eq!(hdr, expected_hdr);
        assert_eq!(body, VALID_REQUEST);
        let fds = fds.unwrap();
        assert_eq!(fds.len(), 1);

        sender_thread.join().expect("Sender failed");
    }

    #[test]
    fn test_gpu_backend_oversized_cursor_message() {
        let (mut frontend, backend) = frontend_backend_pair();

        let cursor_update = VhostUserGpuCursorUpdate {
            pos: VhostUserGpuCursorPos {
                scanout_id: 1,
                x: 10,
                y: 10,
            },
            hot_x: 15,
            hot_y: 15,
            data: [0; 4096],
        };

        let expected_hdr = VhostUserGpuMsgHeader::new(
            GpuBackendReq::CURSOR_UPDATE,
            0,
            size_of_val(&cursor_update) as u32,
        );

        let sender_thread = thread::spawn(move || {
            backend.cursor_update(&cursor_update).unwrap();
        });

        let (hdr, body, fds) = frontend.recv_body::<VhostUserGpuCursorUpdate>().unwrap();

        assert_eq!(hdr, expected_hdr);
        assert_eq!(body, cursor_update);
        assert!(fds.is_none());

        sender_thread.join().expect("Sender failed");
    }

    #[test]
    fn test_gpu_backend_send_oversized_message_with_payload() {
        let (mut frontend, backend) = frontend_backend_pair();

        let expected_payload = vec![1; 8192];
        let expected_hdr = VhostUserGpuMsgHeader::new(
            REQUEST,
            0,
            (size_of_val(&VALID_REQUEST) + expected_payload.len()) as u32,
        );

        let sending_data = expected_payload.clone();
        let sender_thread = thread::spawn(move || {
            backend
                .send_message_oversized_with_payload_no_reply(
                    REQUEST,
                    &VALID_REQUEST,
                    &sending_data,
                    None,
                )
                .unwrap();
        });

        let mut recv_payload = vec![0; 8192];
        let (hdr, body, num_bytes, fds) = frontend
            .recv_payload_into_buf::<MockData>(&mut recv_payload)
            .unwrap();

        assert_eq!(hdr, expected_hdr);
        assert_eq!(body, VALID_REQUEST);
        assert_eq!(num_bytes, expected_payload.len());
        assert!(fds.is_none());
        assert_eq!(&recv_payload[..], &expected_payload[..]);

        sender_thread.join().expect("Sender failed");
    }
}
