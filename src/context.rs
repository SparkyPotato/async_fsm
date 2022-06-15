use std::{alloc::Layout, cell::Cell, ptr::NonNull};

use crate::arena::Arena;

thread_local! {
	static THREAD_CONTEXT: ThreadLocalContext = const {
		ThreadLocalContext {
			arena: Arena::new(),
			input: Cell::new(std::ptr::null_mut()),
		}
	}
}

struct ThreadLocalContext {
	arena: Arena,
	input: Cell<*mut ()>,
}

pub fn alloc_buf(layout: Layout) -> NonNull<[u8]> { THREAD_CONTEXT.with(|ctx| ctx.arena.allocate(layout)) }

/// ## SAFETY
/// `ptr` must have come from a previous call to `alloc_buf` on the same thread.
pub unsafe fn dealloc_buf(ptr: NonNull<u8>) { THREAD_CONTEXT.with(|ctx| ctx.arena.deallocate(ptr)) }

pub fn set_input_ptr(ptr: *mut ()) { THREAD_CONTEXT.with(|ctx| ctx.input.set(ptr)) }

pub fn get_input_ptr() -> *mut () { THREAD_CONTEXT.with(|ctx| ctx.input.get()) }
