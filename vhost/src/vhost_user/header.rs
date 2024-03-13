use crate::vhost_user::message::{Req, VhostUserHeaderFlag, VhostUserMsgValidator, MAX_MSG_SIZE};
use crate::vhost_user::Error;
use std::fmt::Debug;
use std::marker::PhantomData;
use vm_memory::ByteValued;

pub(super) trait MsgHeader: ByteValued + Default + VhostUserMsgValidator {
    type Request: Req;
}

/// Common message header for vhost-user requests and replies.
/// A vhost-user message consists of 3 header fields and an optional payload. All numbers are in the
/// machine native byte order.
#[repr(C, packed)]
#[derive(Copy)]
pub struct VhostUserMsgHeader<R: Req> {
    request: u32,
    flags: u32,
    size: u32,
    _r: PhantomData<R>,
}

impl<R: Req> Debug for VhostUserMsgHeader<R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VhostUserMsgHeader")
            .field("request", &{ self.request })
            .field("flags", &{ self.flags })
            .field("size", &{ self.size })
            .finish()
    }
}

impl<R: Req> Clone for VhostUserMsgHeader<R> {
    fn clone(&self) -> VhostUserMsgHeader<R> {
        *self
    }
}

impl<R: Req> PartialEq for VhostUserMsgHeader<R> {
    fn eq(&self, other: &Self) -> bool {
        self.request == other.request && self.flags == other.flags && self.size == other.size
    }
}

#[allow(dead_code)]
impl<R: Req> VhostUserMsgHeader<R> {
    /// Create a new instance of `VhostUserMsgHeader`.
    pub fn new(request: R, flags: u32, size: u32) -> Self {
        // Default to protocol version 1
        let fl = (flags & VhostUserHeaderFlag::ALL_FLAGS.bits()) | 0x1;
        VhostUserMsgHeader {
            request: request.into(),
            flags: fl,
            size,
            _r: PhantomData,
        }
    }

    /// Get message type.
    pub fn get_code(&self) -> crate::vhost_user::Result<R> {
        R::try_from(self.request).map_err(|_| Error::InvalidMessage)
    }

    /// Set message type.
    pub fn set_code(&mut self, request: R) {
        self.request = request.into();
    }

    /// Get message version number.
    pub fn get_version(&self) -> u32 {
        self.flags & 0x3
    }

    /// Set message version number.
    pub fn set_version(&mut self, ver: u32) {
        self.flags &= !0x3;
        self.flags |= ver & 0x3;
    }

    /// Check whether it's a reply message.
    pub fn is_reply(&self) -> bool {
        (self.flags & VhostUserHeaderFlag::REPLY.bits()) != 0
    }

    /// Mark message as reply.
    pub fn set_reply(&mut self, is_reply: bool) {
        if is_reply {
            self.flags |= VhostUserHeaderFlag::REPLY.bits();
        } else {
            self.flags &= !VhostUserHeaderFlag::REPLY.bits();
        }
    }

    /// Check whether reply for this message is requested.
    pub fn is_need_reply(&self) -> bool {
        (self.flags & VhostUserHeaderFlag::NEED_REPLY.bits()) != 0
    }

    /// Mark that reply for this message is needed.
    pub fn set_need_reply(&mut self, need_reply: bool) {
        if need_reply {
            self.flags |= VhostUserHeaderFlag::NEED_REPLY.bits();
        } else {
            self.flags &= !VhostUserHeaderFlag::NEED_REPLY.bits();
        }
    }

    /// Check whether it's the reply message for the request `req`.
    pub fn is_reply_for(&self, req: &VhostUserMsgHeader<R>) -> bool {
        if let (Ok(code1), Ok(code2)) = (self.get_code(), req.get_code()) {
            self.is_reply() && !req.is_reply() && code1 == code2
        } else {
            false
        }
    }

    /// Get message size.
    pub fn get_size(&self) -> u32 {
        self.size
    }

    /// Set message size.
    pub fn set_size(&mut self, size: u32) {
        self.size = size;
    }
}

impl<R: Req> Default for VhostUserMsgHeader<R> {
    fn default() -> Self {
        VhostUserMsgHeader {
            request: 0,
            flags: 0x1,
            size: 0,
            _r: PhantomData,
        }
    }
}

// SAFETY: Safe because all fields of VhostUserMsgHeader are POD.
unsafe impl<R: Req> ByteValued for VhostUserMsgHeader<R> {}

impl<T: Req> VhostUserMsgValidator for VhostUserMsgHeader<T> {
    #[allow(clippy::if_same_then_else)]
    fn is_valid(&self) -> bool {
        if self.get_code().is_err() {
            return false;
        } else if self.size as usize > MAX_MSG_SIZE {
            return false;
        } else if self.get_version() != 0x1 {
            return false;
        } else if (self.flags & VhostUserHeaderFlag::RESERVED_BITS.bits()) != 0 {
            return false;
        }
        true
    }
}

impl<R: Req> MsgHeader for VhostUserMsgHeader<R> {
    type Request = R;
}
