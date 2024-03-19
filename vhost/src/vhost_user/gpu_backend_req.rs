use crate::vhost_user;
use crate::vhost_user::connection::Endpoint;
use crate::vhost_user::gpu_message::{GpuBackendReq, VhostUserGpuMsgHeader};
use crate::vhost_user::header::VhostUserMsgHeader;
use crate::vhost_user::message::{BackendReq, VhostUserMsgValidator, VhostUserU64};
use crate::vhost_user::Error;
use std::os::fd::RawFd;
use std::os::unix::net::UnixStream;
use std::sync::{Arc, Mutex, MutexGuard};
use std::{io, mem};
use vm_memory::ByteValued;

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

    /// Create a new instance from a `UnixStream` object.
    pub fn from_stream(sock: UnixStream) -> Self {
        Self::new(Endpoint::<VhostUserGpuMsgHeader<GpuBackendReq>>::from_stream(sock))
    }

    /// Mark endpoint as failed with specified error code.
    pub fn set_failed(&self, error: i32) {
        self.node().error = Some(error);
    }
}
