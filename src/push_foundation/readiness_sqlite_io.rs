//! Narrow bridge to SQLite's public main-file I/O methods.

use std::ffi::c_void;
use std::marker::PhantomData;
use std::ptr::{self, NonNull};

use rusqlite::{ffi, Connection};

pub(super) const SQLITE_HEADER_LEN: usize = 100;

/// A SHARED lock obtained through the actual `sqlite3_file` owned by `connection`.
///
/// Before SQL starts, Drop releases the lock. `handoff_to_connection` deliberately
/// leaves it held: SQLite's pager adopts that same VFS lock on its first read, and
/// the connection (configured with exclusive locking mode) owns release thereafter.
pub(super) struct MainFileSharedLock<'connection> {
    file: NonNull<ffi::sqlite3_file>,
    unlock: unsafe extern "C" fn(*mut ffi::sqlite3_file, i32) -> i32,
    release_on_drop: bool,
    _connection: PhantomData<&'connection Connection>,
}

impl MainFileSharedLock<'_> {
    pub(super) fn handoff_to_connection(mut self) {
        self.release_on_drop = false;
    }
}

impl Drop for MainFileSharedLock<'_> {
    fn drop(&mut self) {
        if self.release_on_drop {
            // SAFETY: `file` came from this still-live connection, `unlock` came
            // from its version-1-or-newer public io_methods table, and NONE is a
            // valid downgrade target. Connection close is the final fallback.
            let _ = unsafe { (self.unlock)(self.file.as_ptr(), ffi::SQLITE_LOCK_NONE) };
        }
    }
}

pub(super) fn lock_main_file_and_read_header(
    connection: &Connection,
) -> Result<(MainFileSharedLock<'_>, [u8; SQLITE_HEADER_LEN]), &'static str> {
    let mut file = ptr::null_mut::<ffi::sqlite3_file>();
    // SAFETY: the connection is live for the returned guard lifetime; `main` is
    // NUL-terminated; FILE_POINTER expects a writable sqlite3_file** in pArg.
    let result = unsafe {
        ffi::sqlite3_file_control(
            connection.handle(),
            c"main".as_ptr(),
            ffi::SQLITE_FCNTL_FILE_POINTER,
            (&mut file as *mut *mut ffi::sqlite3_file).cast::<c_void>(),
        )
    };
    if result != ffi::SQLITE_OK {
        return Err("main_file_pointer");
    }
    let file = NonNull::new(file).ok_or("main_file_pointer")?;

    // SAFETY: FILE_POINTER succeeded with a non-null file. SQLite guarantees
    // pMethods is the public method table for that open file while connection lives.
    let methods = unsafe { file.as_ref().pMethods };
    let methods = NonNull::new(methods.cast_mut()).ok_or("main_io_methods")?;
    // SAFETY: the non-null method table belongs to the live sqlite3_file.
    let methods = unsafe { methods.as_ref() };
    if methods.iVersion < 1 {
        return Err("main_io_methods_version");
    }
    let lock = methods.xLock.ok_or("main_xlock")?;
    let unlock = methods.xUnlock.ok_or("main_xunlock")?;
    let read = methods.xRead.ok_or("main_xread")?;

    // SAFETY: method pointers and file are from the same live public VFS table.
    let lock_result = unsafe { lock(file.as_ptr(), ffi::SQLITE_LOCK_SHARED) };
    if lock_result != ffi::SQLITE_OK {
        // SQLite documents xLock failures as leaving no stronger usable lock;
        // best-effort cleanup plus connection close prevents a leaked lock.
        let _ = unsafe { unlock(file.as_ptr(), ffi::SQLITE_LOCK_NONE) };
        return Err("main_shared_lock");
    }
    let guard = MainFileSharedLock {
        file,
        unlock,
        release_on_drop: true,
        _connection: PhantomData,
    };
    let mut header = [0_u8; SQLITE_HEADER_LEN];
    // SAFETY: header is writable for exactly SQLITE_HEADER_LEN bytes, offset 0
    // is valid, and xRead belongs to `file`. Short/corrupt files fail closed.
    let read_result = unsafe {
        read(
            file.as_ptr(),
            header.as_mut_ptr().cast::<c_void>(),
            SQLITE_HEADER_LEN as i32,
            0,
        )
    };
    if read_result != ffi::SQLITE_OK {
        return Err("main_header_read");
    }
    Ok((guard, header))
}
