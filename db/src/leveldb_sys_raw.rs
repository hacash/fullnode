use std::ffi::{CStr, CString, c_char, c_void};
use std::path::Path;
use std::ptr;
use std::slice;
use std::str::from_utf8;

use leveldb_sys::*;
use libc::size_t;
use sys::Rerr;

use crate::config::db_sync_enabled;

fn new_string_from_char_ptr(message: *const c_char) -> String {
    unsafe {
        let s = from_utf8(CStr::from_ptr(message).to_bytes())
            .unwrap()
            .to_string();
        leveldb_free(message as *mut c_void);
        s
    }
}

/// Bytes allocated by leveldb
///
/// It's basically the same thing as `Box<[u8]>` except that it uses
/// leveldb_free() as a destructor.
pub struct RawBytes {
    // We use static reference instead of pointer to inform the compiler that
    // it can't be null. (Because `NonZero` is unstable now.)
    bytes: &'static mut u8,
    size: usize,
}

impl RawBytes {
    /// Creates instance of `RawBytes` from leveldb-allocated data.
    ///
    /// Returns `None` if `ptr` is `null`.
    pub unsafe fn from_raw(ptr: *mut u8, size: usize) -> Option<Self> {
        if ptr.is_null() {
            None
        } else {
            unsafe {
                Some(RawBytes {
                    bytes: &mut *ptr,
                    size: size,
                })
            }
        }
    }

    /// Creates instance of `RawBytes` from leveldb-allocated data without null checking.
    pub unsafe fn from_raw_unchecked(ptr: *mut u8, size: usize) -> Self {
        unsafe {
            RawBytes {
                bytes: &mut *ptr,
                size: size,
            }
        }
    }
}

impl Drop for RawBytes {
    fn drop(&mut self) {
        unsafe {
            leveldb_sys::leveldb_free(self.bytes as *mut u8 as *mut c_void);
        }
    }
}

impl ::std::ops::Deref for RawBytes {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        unsafe { slice::from_raw_parts(self.bytes, self.size) }
    }
}

impl ::std::ops::DerefMut for RawBytes {
    fn deref_mut(&mut self) -> &mut Self::Target {
        unsafe { slice::from_raw_parts_mut(self.bytes as *mut u8, self.size) }
    }
}

impl ::std::borrow::Borrow<[u8]> for RawBytes {
    fn borrow(&self) -> &[u8] {
        &*self
    }
}

impl ::std::borrow::BorrowMut<[u8]> for RawBytes {
    fn borrow_mut(&mut self) -> &mut [u8] {
        &mut *self
    }
}

impl AsRef<[u8]> for RawBytes {
    fn as_ref(&self) -> &[u8] {
        &*self
    }
}

impl AsMut<[u8]> for RawBytes {
    fn as_mut(&mut self) -> &mut [u8] {
        &mut *self
    }
}

impl From<RawBytes> for Vec<u8> {
    fn from(bytes: RawBytes) -> Self {
        bytes.as_ref().to_owned()
    }
}

impl From<RawBytes> for Box<[u8]> {
    fn from(bytes: RawBytes) -> Self {
        bytes.as_ref().to_owned().into_boxed_slice()
    }
}

struct RawDB {
    ptr: *mut leveldb_t,
}

impl Drop for RawDB {
    fn drop(&mut self) {
        unsafe {
            leveldb_close(self.ptr);
        }
    }
}

unsafe impl Send for RawDB {}
unsafe impl Sync for RawDB {}

struct RawReadOptions {
    ptr: *mut leveldb_readoptions_t,
}

impl Drop for RawReadOptions {
    fn drop(&mut self) {
        unsafe {
            leveldb_readoptions_destroy(self.ptr);
        }
    }
}

unsafe impl Send for RawReadOptions {}
unsafe impl Sync for RawReadOptions {}

struct RawWriteOptions {
    ptr: *mut leveldb_writeoptions_t,
}

impl Drop for RawWriteOptions {
    fn drop(&mut self) {
        unsafe {
            leveldb_writeoptions_destroy(self.ptr);
        }
    }
}

unsafe impl Send for RawWriteOptions {}
unsafe impl Sync for RawWriteOptions {}

struct RawIter {
    ptr: *mut leveldb_iterator_t,
}

impl Drop for RawIter {
    fn drop(&mut self) {
        unsafe {
            leveldb_iter_destroy(self.ptr);
        }
    }
}

unsafe extern "C" {
    #[link_name = "leveldb_iter_get_error"]
    fn leveldb_iter_get_error_mut(it: *const leveldb_iterator_t, errptr: *mut *mut c_char);
}

pub struct Writebatch {
    length: usize,
    pub ptr: *mut leveldb_writebatch_t,
}

impl Drop for Writebatch {
    fn drop(&mut self) {
        unsafe {
            leveldb_writebatch_destroy(self.ptr);
        }
    }
}

impl Writebatch {
    pub fn new() -> Writebatch {
        let ptr = unsafe { leveldb_writebatch_create() };
        Writebatch { ptr, length: 0 }
    }

    #[allow(dead_code)]
    pub fn len(&self) -> usize {
        self.length
    }
    pub fn put(&mut self, k: &[u8], value: &[u8]) {
        self.length += 1;
        unsafe {
            leveldb_writebatch_put(
                self.ptr,
                k.as_ptr() as *mut c_char,
                k.len() as size_t,
                value.as_ptr() as *mut c_char,
                value.len() as size_t,
            );
        }
    }

    pub fn delete(&mut self, k: &[u8]) {
        unsafe {
            leveldb_writebatch_delete(self.ptr, k.as_ptr() as *mut c_char, k.len() as size_t);
        }
    }

    #[allow(dead_code)]
    pub fn deref(self) -> Self {
        self
    }
}

pub struct LevelDB {
    database: RawDB,
    read_options: RawReadOptions,
    write_options: RawWriteOptions,
    // ldb: LevelDatabase<LDBKey>,
}

impl LevelDB {
    pub fn open(dir: &Path) -> LevelDB {
        let mut error = ptr::null_mut();
        let database = unsafe {
            let c_options = leveldb_options_create();
            leveldb_options_set_create_if_missing(c_options, 1u8);
            let c_dbpath = CString::new(dir.to_str().unwrap()).unwrap();
            let db = leveldb_open(
                c_options as *const leveldb_options_t,
                c_dbpath.as_bytes_with_nul().as_ptr() as *const c_char,
                &mut error,
            );
            leveldb_options_destroy(c_options);
            db
        };
        if error != ptr::null_mut() {
            let err = new_string_from_char_ptr(error);
            panic!("{}", err)
        }
        let read_options = unsafe {
            RawReadOptions {
                ptr: leveldb_readoptions_create(),
            }
        };
        let write_options = unsafe {
            let ptr = leveldb_writeoptions_create();
            leveldb_writeoptions_set_sync(ptr, if db_sync_enabled() { 1 } else { 0 });
            RawWriteOptions { ptr }
        };
        LevelDB {
            database: RawDB { ptr: database },
            read_options,
            write_options,
        }
    }

    pub fn get_at(&self, k: &[u8]) -> Option<RawBytes> {
        let mut error = ptr::null_mut();
        let mut length: size_t = 0;
        let result = unsafe {
            let res = leveldb_get(
                self.database.ptr,
                self.read_options.ptr,
                k.as_ptr() as *mut c_char,
                k.len() as size_t,
                &mut length,
                &mut error,
            );
            res
        };
        if error != ptr::null_mut() {
            let err = new_string_from_char_ptr(error);
            panic!("{}", err)
        }
        if result.is_null() {
            return None; // key not found
        }
        Some(unsafe { RawBytes::from_raw_unchecked(result as *mut u8, length) })
    }

    pub fn get(&self, k: &[u8]) -> Option<Vec<u8>> {
        if let Some(v) = self.get_at(k) {
            return Some(v.into());
        }
        None
    }

    pub fn put(&self, k: &[u8], value: &[u8]) {
        let mut error = ptr::null_mut();
        unsafe {
            leveldb_put(
                self.database.ptr,
                self.write_options.ptr,
                k.as_ptr() as *mut c_char,
                k.len() as size_t,
                value.as_ptr() as *mut c_char,
                value.len() as size_t,
                &mut error,
            );
        }
        if error != ptr::null_mut() {
            let err = new_string_from_char_ptr(error);
            panic!("{}", err)
        }
    }

    pub fn rm(&self, k: &[u8]) {
        let mut error = ptr::null_mut();
        unsafe {
            leveldb_delete(
                self.database.ptr,
                self.write_options.ptr,
                k.as_ptr() as *mut c_char,
                k.len() as size_t,
                &mut error,
            );
        }
        if error != ptr::null_mut() {
            let err = new_string_from_char_ptr(error);
            panic!("{}", err)
        }
    }

    pub fn try_write(&self, batch: &Writebatch) -> Rerr {
        let mut error = ptr::null_mut();
        unsafe {
            leveldb_write(
                self.database.ptr,
                self.write_options.ptr,
                batch.ptr,
                &mut error,
            );
        }
        if error != ptr::null_mut() {
            let err = new_string_from_char_ptr(error);
            return Err(sys::Error::fault(err));
        }
        Ok(())
    }

    pub fn for_each(&self, each: &mut dyn FnMut(&[u8], &[u8])) -> Rerr {
        let iter = unsafe {
            let ptr = leveldb_create_iterator(self.database.ptr, self.read_options.ptr);
            leveldb_iter_seek_to_first(ptr);
            RawIter { ptr }
        };
        loop {
            if unsafe { leveldb_iter_valid(iter.ptr) } == 0 {
                break;
            }
            let mut klen: size_t = 0;
            let mut vlen: size_t = 0;
            let (kptr, vptr) = unsafe {
                (
                    leveldb_iter_key(iter.ptr, &mut klen),
                    leveldb_iter_value(iter.ptr, &mut vlen),
                )
            };
            let (k, v) = unsafe {
                (
                    ::std::slice::from_raw_parts(kptr as *const u8, klen as usize),
                    ::std::slice::from_raw_parts(vptr as *const u8, vlen as usize),
                )
            };
            each(k, v);
            unsafe {
                leveldb_iter_next(iter.ptr);
            }
        }
        let mut error: *mut c_char = ptr::null_mut();
        unsafe {
            leveldb_iter_get_error_mut(iter.ptr as *const leveldb_iterator_t, &mut error);
        }
        if !error.is_null() {
            return Err(new_string_from_char_ptr(error as *const c_char).into());
        }
        Ok(())
    }
}
