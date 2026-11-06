//! Where a plan's arrays live: in blocks of its own, or read in place from its converted file
//! (`<bundle>.plan`) mapped into memory (Windows), so the pages are the file's: shared by every run of
//! the same build that reads the bundle and handed back by the system under pressure, not the run's own
//! (a profiled scene's bundle plans were 60-90 MB of each run's memory, a mesh effects scene's
//! ~365 MB). A mapped file is never written in place: the cache replaces a plan file by renaming a new
//! one over it, which leaves a mapped view reading the old bytes.

use std::sync::Arc;

/// A plain array element: `#[repr(C)]`, made only of 4-byte numbers (or bytes), without padding, valid
/// for any bits, so its bytes are its little-endian file encoding.
///
/// # Safety
/// The type is `#[repr(C)]`, has no padding and no invalid bit patterns, and its alignment is at most 4.
pub(crate) unsafe trait Pod: Copy + 'static {}

unsafe impl Pod for u8 {}
unsafe impl Pod for u32 {}
unsafe impl Pod for f32 {}

/// A plan file mapped read-only.
pub(crate) struct Mapping {
    base: *const u8,
    len: usize,
}

// (read-only memory: any thread may read it)
unsafe impl Send for Mapping {}
unsafe impl Sync for Mapping {}

impl Mapping {
    /// `path` mapped read-only, or None (empty files, other systems, any failure).
    #[cfg(all(windows, target_endian = "little"))]
    pub(crate) fn open(path: &std::path::Path) -> Option<Arc<Mapping>> {
        use std::os::windows::io::AsRawHandle;
        type Handle = *mut std::ffi::c_void;
        unsafe extern "system" {
            fn CreateFileMappingW(file: Handle, attributes: *const std::ffi::c_void, protect: u32, high: u32, low: u32, name: *const u16) -> Handle;
            fn MapViewOfFile(mapping: Handle, access: u32, high: u32, low: u32, bytes: usize) -> *mut std::ffi::c_void;
            fn CloseHandle(handle: Handle) -> i32;
        }
        const PAGE_READONLY: u32 = 0x02;
        const FILE_MAP_READ: u32 = 0x04;
        let file = std::fs::File::open(path).ok()?;
        let len = usize::try_from(file.metadata().ok()?.len()).ok()?;
        if len == 0 {
            return None;
        }
        unsafe {
            let mapping = CreateFileMappingW(file.as_raw_handle() as Handle, std::ptr::null(), PAGE_READONLY, 0, 0, std::ptr::null());
            if mapping.is_null() {
                return None;
            }
            // (the view keeps the mapping and the file open)
            let base = MapViewOfFile(mapping, FILE_MAP_READ, 0, 0, 0);
            CloseHandle(mapping);
            if base.is_null() {
                return None;
            }
            Some(Arc::new(Mapping { base: base as *const u8, len }))
        }
    }

    #[cfg(not(all(windows, target_endian = "little")))]
    pub(crate) fn open(_path: &std::path::Path) -> Option<Arc<Mapping>> {
        None
    }

    pub(crate) fn bytes(&self) -> &[u8] {
        unsafe { std::slice::from_raw_parts(self.base, self.len) }
    }
}

impl Drop for Mapping {
    fn drop(&mut self) {
        #[cfg(all(windows, target_endian = "little"))]
        unsafe {
            unsafe extern "system" {
                fn UnmapViewOfFile(base: *const std::ffi::c_void) -> i32;
            }
            UnmapViewOfFile(self.base as *const std::ffi::c_void);
        }
    }
}

/// One of a plan's arrays: its own block, or a range of its mapped file.
pub(crate) enum Array<T: Pod> {
    Owned(Box<[T]>),
    Mapped { file: Arc<Mapping>, offset: usize, len: usize },
}

impl<T: Pod> Array<T> {
    /// `len` items at byte `offset` of `bytes` (in range, `offset` a multiple of 4): a range of `file`
    /// when the bytes are its own, else copied.
    pub(crate) fn read(bytes: &[u8], file: Option<&Arc<Mapping>>, offset: usize, len: usize) -> Array<T> {
        let size = len * std::mem::size_of::<T>();
        debug_assert!(offset + size <= bytes.len() && offset % std::mem::align_of::<T>() == 0);
        match file {
            Some(file) if std::ptr::eq(file.bytes().as_ptr(), bytes.as_ptr()) && (file.base as usize + offset) % std::mem::align_of::<T>() == 0 => {
                Array::Mapped { file: file.clone(), offset, len }
            }
            _ => {
                let mut items: Vec<T> = Vec::with_capacity(len);
                unsafe {
                    std::ptr::copy_nonoverlapping(bytes[offset..offset + size].as_ptr(), items.as_mut_ptr() as *mut u8, size);
                    items.set_len(len);
                }
                Array::Owned(items.into_boxed_slice())
            }
        }
    }

    /// The array in a block of its exact size made now (a mapped one stays mapped).
    pub(crate) fn compacted(&self) -> Array<T> {
        match self {
            Array::Owned(items) => {
                let mut fresh = Vec::with_capacity(items.len());
                fresh.extend_from_slice(items);
                Array::Owned(fresh.into_boxed_slice())
            }
            Array::Mapped { file, offset, len } => Array::Mapped { file: file.clone(), offset: *offset, len: *len },
        }
    }

    /// The items' bytes: their little-endian encoding (see [`Pod`]).
    pub(crate) fn bytes(&self) -> &[u8] {
        let items: &[T] = self;
        unsafe { std::slice::from_raw_parts(items.as_ptr() as *const u8, std::mem::size_of_val(items)) }
    }
}

impl<T: Pod> std::ops::Deref for Array<T> {
    type Target = [T];

    #[inline]
    fn deref(&self) -> &[T] {
        match self {
            Array::Owned(items) => items,
            Array::Mapped { file, offset, len } => unsafe { std::slice::from_raw_parts(file.base.add(*offset) as *const T, *len) },
        }
    }
}

impl<T: Pod> Clone for Array<T> {
    fn clone(&self) -> Self {
        match self {
            Array::Owned(items) => Array::Owned(items.clone()),
            Array::Mapped { file, offset, len } => Array::Mapped { file: file.clone(), offset: *offset, len: *len },
        }
    }
}

impl<T: Pod> Default for Array<T> {
    fn default() -> Self {
        Array::Owned(Box::new([]))
    }
}

impl<T: Pod> From<Vec<T>> for Array<T> {
    fn from(items: Vec<T>) -> Self {
        Array::Owned(items.into_boxed_slice())
    }
}

impl<T: Pod + std::fmt::Debug> std::fmt::Debug for Array<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_list().entries(self.iter()).finish()
    }
}
