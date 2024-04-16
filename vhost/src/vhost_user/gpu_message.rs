//! Implementation parts of the protocol on the socket from VHOST_USER_SET_GPU_SOCKET
//! see: https://www.qemu.org/docs/master/interop/vhost-user-gpu.html

use crate::vhost_user::message::{enum_value, MsgHeader, Req, VhostUserMsgValidator};
use crate::vhost_user::Error;
use std::fmt::Debug;
use std::marker::PhantomData;
use vm_memory::ByteValued;

enum_value! {
    /// Type of requests sending from gpu backends to gpu frontends.
    #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
    #[allow(non_camel_case_types)]
    pub enum GpuBackendReq: u32 {
        /// Get the supported protocol features bitmask.
        GET_PROTOCOL_FEATURES = 1,
        /// Enable protocol features using a bitmask.
        SET_PROTOCOL_FEATURES = 2,
        /// Get the preferred display configuration.
        GET_DISPLAY_INFO = 3,
        /// Set/show the cursor position.
        CURSOR_POS = 4,
        /// Set/hide the cursor.
        CURSOR_POS_HIDE = 5,
        /// Set/show the cursor position update.
        CURSOR_POS_UPDATE = 6,
        /// Set the scanout resolution.
        /// To disable a scanout, the dimensions width/height are set to 0.
        SCANOUT = 7,
        /// Update the scanout content. The data payload contains the graphical bits.
        /// The display should be flushed and presented.
        UPDATE = 8,
        /// Set the scanout resolution/configuration, and share a DMABUF file descriptor for the scanout content,
        /// which is passed as ancillary data.
        /// To disable a scanout, the dimensions width/height are set to 0, there is no file descriptor passed.
        DMABUF_SCANOUT = 9,
        /// The display should be flushed and presented according to updated region from VhostUserGpuUpdate.
        // Note: there is no data payload, since the scanout is shared thanks to DMABUF,
        // that must have been set previously with VHOST_USER_GPU_DMABUF_SCANOUT.
        DMABUF_UPDATE = 10,
        /// Retrieve the EDID data for a given scanout.
        /// This message requires the VHOST_USER_GPU_PROTOCOL_F_EDID protocol feature to be supported.
        GET_EDID = 11,
        /// Same as VHOST_USER_GPU_DMABUF_SCANOUT, but also sends the dmabuf modifiers appended to the message,
        /// which were not provided in the other message.
        /// This message requires the VHOST_USER_GPU_PROTOCOL_F_DMABUF2 protocol feature to be supported.
        VHOST_USER_GPU_DMABUF_SCANOUT2 = 12,
    }
}

impl Req for GpuBackendReq {}

// Bit mask for common message flags.
bitflags! {
    /// Common message flags for vhost-user requests and replies.
    pub struct VhostUserGpuHeaderFlag: u32 {
        /// Mark message as reply.
        const REPLY = 0x4;
    }
}

/// A vhost-user message consists of 3 header fields and an optional payload. All numbers are in the
/// machine native byte order.
#[repr(C, packed)]
#[derive(Copy)]
pub struct VhostUserGpuMsgHeader<R: Req> {
    request: u32,
    flags: u32,
    size: u32,
    _r: PhantomData<R>,
}

impl<R: Req> Debug for VhostUserGpuMsgHeader<R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VhostUserMsgHeader")
            .field("request", &{ self.request })
            .field("flags", &{ self.flags })
            .field("size", &{ self.size })
            .finish()
    }
}

impl<R: Req> Clone for VhostUserGpuMsgHeader<R> {
    fn clone(&self) -> VhostUserGpuMsgHeader<R> {
        *self
    }
}

impl<R: Req> PartialEq for VhostUserGpuMsgHeader<R> {
    fn eq(&self, other: &Self) -> bool {
        self.request == other.request && self.flags == other.flags && self.size == other.size
    }
}

#[allow(dead_code)]
impl<R: Req> VhostUserGpuMsgHeader<R> {
    /// Create a new instance of `VhostUserMsgHeader`.
    pub fn new(request: R, flags: u32, size: u32) -> Self {
        VhostUserGpuMsgHeader {
            request: request.into(),
            flags: flags, // TODO: maybe check if the flags are valid?
            size,
            _r: PhantomData,
        }
    }

    /// Get message type.
    pub fn get_code(&self) -> crate::vhost_user::Result<R> {
        R::try_from(self.request).map_err(|_| Error::InvalidMessage)
    }

    /// Check whether it's a reply message.
    pub fn is_reply(&self) -> bool {
        (self.flags & VhostUserGpuHeaderFlag::REPLY.bits()) != 0
    }

    /// Mark message as reply.
    pub fn set_reply(&mut self, is_reply: bool) {
        if is_reply {
            self.flags |= VhostUserGpuHeaderFlag::REPLY.bits();
        } else {
            self.flags &= !VhostUserGpuHeaderFlag::REPLY.bits();
        }
    }

    /// Check whether it's the reply message for the request `req`.
    pub fn is_reply_for(&self, req: &VhostUserGpuMsgHeader<R>) -> bool {
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

impl<R: Req> Default for VhostUserGpuMsgHeader<R> {
    fn default() -> Self {
        VhostUserGpuMsgHeader {
            request: 0,
            flags: 0x1,
            size: 0,
            _r: PhantomData,
        }
    }
}

// SAFETY: Safe because all fields of VhostUserMsgHeader are POD.
unsafe impl<R: Req> ByteValued for VhostUserGpuMsgHeader<R> {}

impl<T: Req> VhostUserMsgValidator for VhostUserGpuMsgHeader<T> {
    #[allow(clippy::if_same_then_else)]
    fn is_valid(&self) -> bool {
        self.get_code().is_ok()
    }
}

impl<R: Req> MsgHeader for VhostUserGpuMsgHeader<R> {
    type Request = R;
}

/* data passed in the control vq, 2d related */

/// The rectangle represents the portion of the blob resource being displayed.
#[derive(Copy, Clone, Debug, Default)]
#[repr(C)]
pub struct VirtioGpuRect {
    /// The position field x describe how the displays are arranged
    pub x: u32,
    /// The position field y describe how the displays are arranged
    pub y: u32,
    /// The size field width
    pub width: u32,
    /// The size field height
    pub height: u32,
}
unsafe impl ByteValued for VirtioGpuRect {}

/// The Structure of virtio_gpu_display_one
#[derive(Copy, Clone, Debug, Default)]
#[repr(C)]
pub struct VirtioGpuDisplayOne {
    /// The structure of the rectangle
    pub r: VirtioGpuRect,
    /// The enabled field is set when the user enabled the display.
    pub enabled: u32,
    /// The display flag
    pub flags: u32,
}
unsafe impl ByteValued for VirtioGpuDisplayOne {}

/// The fixed header struct virtio_gpu_ctrl_hdr in each request
#[derive(Copy, Clone, Debug, Default)]
#[repr(C)]
pub struct VirtioGpuCtrlHdr {
    /// Specifies the type of the driver request (VIRTIO_GPU_CMD_*)
    /// or device response (VIRTIO_GPU_RESP_*).
    pub type_: u32,
    /// Request / response flags.
    pub flags: u32,
    /// Set VIRTIO_GPU_FLAG_FENCE bit in the response
    pub fence_id: u64,
    /// Rendering context (used in 3D mode only).
    pub ctx_id: u32,
    /// ring_idx indicates the value of a context-specific ring index.
    /// The minimum value is 0 and maximum value is 63 (inclusive).
    pub ring_idx: u8,
    /// padding of the structure
    pub padding: [u8; 3],
}
unsafe impl ByteValued for VirtioGpuCtrlHdr {}

/// VIRTIO_GPU_MAX_SCANOUTS data
pub const VIRTIO_GPU_MAX_SCANOUTS: usize = 16;

/* VIRTIO_GPU_RESP_OK_DISPLAY_INFO */
///The response contains a list of per-scanout information.
#[derive(Copy, Clone, Debug, Default)]
#[repr(C)]
pub struct VirtioGpuRespDisplayInfo {
    /// The fixed header struct
    pub hdr: VirtioGpuCtrlHdr,
    /// pmodes contains whether the scanout is enabled and what
    /// its preferred position and size is
    pub pmodes: [VirtioGpuDisplayOne; VIRTIO_GPU_MAX_SCANOUTS],
}
unsafe impl ByteValued for VirtioGpuRespDisplayInfo {}

impl VhostUserMsgValidator for VirtioGpuRespDisplayInfo {}

/// Retrieve the EDID data for a given scanout
/// Request data is struct VhostUserGpuEdidRequest
#[derive(Copy, Clone, Debug, Default)]
#[repr(C)]
pub struct VhostUserGpuEdidRequest {
    /// scanout information
    pub scanout_id: u32,
}
unsafe impl ByteValued for VhostUserGpuEdidRequest {}

impl VhostUserMsgValidator for VhostUserGpuEdidRequest {}

/// Set the scanout resolution.
/// Data is struct VhostUserGpuScanout
#[derive(Copy, Clone, Debug, Default)]
#[repr(C)]
pub struct VhostUserGpuScanout {
    /// scanout information
    pub scanout_id: u32,
    /// scanout width size
    pub width: u32,
    /// scanout height size
    pub height: u32,
}
unsafe impl ByteValued for VhostUserGpuScanout {}

impl VhostUserMsgValidator for VhostUserGpuScanout {}

/// Set the scanout resolution.
/// Data is struct VhostUserGpuDMABUFScanout
#[derive(Copy, Clone, Debug, Default)]
#[repr(C)]
pub struct VhostUserGpuDMABUFScanout {
    /// scanout information
    pub scanout_id: u32,
    /// The position field x of the scanout within the DMABUF
    pub x: u32,
    /// The position field y of the scanout within the DMABUF
    pub y: u32,
    /// scanout width size
    pub width: u32,
    /// scanout height size
    pub height: u32,
    /// the DMABUF width
    pub fd_width: u32,
    /// the DMABUF height
    pub fd_height: u32,
    /// the DMABUF stride
    pub fd_stride: u32,
    /// the DMABUF flags
    pub fd_flags: u32,
    /// the DMABUF fourcc
    pub fd_drm_fourcc: u32,
}
unsafe impl ByteValued for VhostUserGpuDMABUFScanout {}

impl VhostUserMsgValidator for VhostUserGpuDMABUFScanout {}

/// Update the scanout content.
/// Data is struct VhostUserGpuUpdate
#[derive(Copy, Clone, Debug, Default)]
#[repr(C)]
pub struct VhostUserGpuUpdate {
    /// scanout content to update
    pub scanout_id: u32,
    /// The position field x of the scanout update
    pub x: u32,
    /// The position field y of the scanout update
    pub y: u32,
    /// scanout updated width size
    pub width: u32,
    /// scanout updated height size
    pub height: u32,
}
unsafe impl ByteValued for VhostUserGpuUpdate {}

impl VhostUserMsgValidator for VhostUserGpuUpdate {}

/// The cursor location
/// Data is struct VhostUserGpuCursorPos
#[derive(Copy, Clone, Debug, Default)]
#[repr(C)]
pub struct VhostUserGpuCursorPos {
    /// the scanout where the cursor is located
    pub scanout_id: u32,
    /// The cursor position field x
    pub x: u32,
    /// The cursor position field y
    pub y: u32,
}
unsafe impl ByteValued for VhostUserGpuCursorPos {}

impl VhostUserMsgValidator for VhostUserGpuCursorPos {}

/// Update the cursor shape and location.
/// Data is struct VhostUserGpuCursorUpdate
#[derive(Copy, Clone, Debug)]
#[repr(C)]
pub struct VhostUserGpuCursorUpdate {
    /// The cursor location
    pub pos: VhostUserGpuCursorPos,
    /// The cursor hot location x
    pub hot_x: u32,
    /// The cursor hot location y
    pub hot_y: u32,
    /// 64x64 RGBA cursor data
    pub data: [u32; 64 * 64],
}

impl Default for VhostUserGpuCursorUpdate {
    fn default() -> Self {
        VhostUserGpuCursorUpdate {
            pos: VhostUserGpuCursorPos::default(),
            hot_x: u32::default(),
            hot_y: u32::default(),
            data: [0; 64 * 64],
        }
    }
}

unsafe impl ByteValued for VhostUserGpuCursorUpdate {}

impl VhostUserMsgValidator for VhostUserGpuCursorUpdate {}

/* VIRTIO_GPU_RESP_OK_EDID */
/// Response type is VIRTIO_GPU_RESP_OK_EDID
#[derive(Copy, Clone, Debug)]
#[repr(C)]
pub struct VirtioGpuRespGetEdid {
    /// The fixed header struct
    pub hdr: VirtioGpuCtrlHdr,
    /// size of the structure containing information
    pub size: u32,
    /// padding of the structure
    pub padding: u32,
    /// The EDID display data blob (as specified by VESA) for the scanout.
    pub edid: [u8; 1024],
}
unsafe impl ByteValued for VirtioGpuRespGetEdid {}

impl Default for VirtioGpuRespGetEdid {
    fn default() -> Self {
        VirtioGpuRespGetEdid {
            hdr: VirtioGpuCtrlHdr::default(),
            size: u32::default(),
            padding: u32::default(),
            edid: [0; 1024], // Default value for the edid array (filled with zeros)
        }
    }
}

impl VhostUserMsgValidator for VirtioGpuRespGetEdid {}
