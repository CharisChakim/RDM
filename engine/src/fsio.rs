use std::fs::File;
use std::io;

/// Positioned write, safe to call from several threads on the same handle.
#[cfg(windows)]
pub fn write_at(f: &File, mut buf: &[u8], mut off: u64) -> io::Result<()> {
    use std::os::windows::fs::FileExt;
    while !buf.is_empty() {
        let n = f.seek_write(buf, off)?;
        buf = &buf[n..];
        off += n as u64;
    }
    Ok(())
}

#[cfg(unix)]
pub fn write_at(f: &File, buf: &[u8], off: u64) -> io::Result<()> {
    use std::os::unix::fs::FileExt;
    f.write_all_at(buf, off)
}

/// Without this, NTFS zero-fills everything before the first write at a far offset,
/// which stalls the connection that writes the tail of a multi-GB file.
#[cfg(windows)]
pub fn set_sparse(f: &File) {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::System::{Ioctl::FSCTL_SET_SPARSE, IO::DeviceIoControl};
    let mut ret = 0u32;
    // Best effort: FAT/exFAT don't support sparse files and the download still works.
    unsafe {
        DeviceIoControl(
            f.as_raw_handle() as _,
            FSCTL_SET_SPARSE,
            std::ptr::null(),
            0,
            std::ptr::null_mut(),
            0,
            &mut ret,
            std::ptr::null_mut(),
        );
    }
}

#[cfg(unix)]
pub fn set_sparse(_f: &File) {}
