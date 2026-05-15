// font-kit/c/src/lib.rs
//
// Copyright © 2019 The Pathfinder Project Developers.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

use font_kit::handle::Handle;
use std::mem;
use std::slice;
use std::sync::Arc;

pub type FKDataRef = *const Vec<u8>;
pub type FKHandleRef = *mut Handle;

/// Copies raw bytes into a new font-kit data buffer.
///
/// # Safety
///
/// `bytes` must be valid for reads of `len` bytes and properly aligned. The pointer must be
/// non-null even when `len` is zero.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn FKDataCreate(bytes: *const u8, len: usize) -> FKDataRef {
    Arc::into_raw(Arc::new(unsafe {
        slice::from_raw_parts(bytes, len).to_vec()
    }))
}

/// Destroys a font-kit data buffer.
///
/// # Safety
///
/// `data` must be a pointer returned by `FKDataCreate` and must not have been destroyed already.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn FKDataDestroy(data: FKDataRef) {
    drop(unsafe { Arc::from_raw(data) })
}

/// Does not take ownership of `bytes`.
///
/// # Safety
///
/// `bytes` must be a valid pointer returned by `FKDataCreate` and must remain alive until the
/// returned handle is destroyed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn FKHandleCreateWithMemory(
    bytes: FKDataRef,
    font_index: u32,
) -> FKHandleRef {
    let bytes = unsafe { Arc::from_raw(bytes) };
    mem::forget(bytes.clone());
    Box::into_raw(Box::new(Handle::from_memory(bytes, font_index)))
}

/// Destroys a font-kit handle.
///
/// # Safety
///
/// `handle` must be a pointer returned by `FKHandleCreateWithMemory` and must not have been
/// destroyed already.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn FKHandleDestroy(handle: FKHandleRef) {
    drop(unsafe { Box::from_raw(handle) })
}
