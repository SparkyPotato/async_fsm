use std::{
	alloc::Layout,
	future::Future,
	pin::Pin,
	ptr::{DynMetadata, NonNull},
};

use crate::context::{alloc_buf, dealloc_buf};

pub struct DynFuture<'a, O> {
	buf: NonNull<[u8]>,
	metadata: Option<DynMetadata<dyn Future<Output = O> + 'a>>,
}

impl<'a, O> DynFuture<'a, O> {
	pub fn new(size: usize, align: usize) -> Self {
		Self {
			buf: alloc_buf(Layout::from_size_align(size, align).unwrap()),
			metadata: None,
		}
	}

	/// ## SAFETY
	/// `self` must have been created with the appropriate size and alignment to store T.
	pub unsafe fn write<T: Future<Output = O> + 'a>(&mut self, future: T) {
		// Call `take` so that we become uninhabited if anything panics.
		if let Some(metadata) = self.metadata.take() {
			// SAFETY: Previous call to `write` ensures that metadata is correct.
			std::ptr::drop_in_place(self.ptr(metadata));
		}
		self.metadata = Some(std::ptr::metadata(&future as &dyn Future<Output = O>));
		// SAFETY: `buf` is valid, and we have dropped the last value that was stored in it.
		(self.buf.as_ptr() as *mut T).write(future);
	}

	pub fn get(&mut self) -> Pin<&mut (dyn Future<Output = O> + 'a)> {
		// SAFETY: `buf` is valid, and we do not deallocate or reuse it without dropping the stored future.
		unsafe { Pin::new_unchecked(&mut *self.ptr(self.metadata.expect("DynFuture was uninhabited"))) }
	}

	unsafe fn ptr(&self, metadata: DynMetadata<dyn Future<Output = O> + 'a>) -> *mut (dyn Future<Output = O> + 'a) {
		std::ptr::from_raw_parts_mut(self.buf.as_ptr() as _, metadata)
	}
}

impl<O> Drop for DynFuture<'_, O> {
	fn drop(&mut self) {
		if let Some(metadata) = self.metadata.take() {
			// SAFETY: Previous call to `write` ensures that metadata is correct.
			unsafe {
				std::ptr::drop_in_place(self.ptr(metadata));
			}
			// SAFETY: `buf` is valid, and we have dropped the last value that was stored in it.
			// `buf` was also obtained from `alloc_buf`.
			unsafe {
				dealloc_buf(self.buf.cast());
			}
		}
	}
}
