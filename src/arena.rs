use std::{
	alloc::{handle_alloc_error, Layout},
	cell::UnsafeCell,
	ptr::NonNull,
};

#[derive(Copy, Clone)]
struct BlockPtr {
	ptr: NonNull<BlockHeader>,
	size: usize,
}

#[repr(align(8))]
struct BlockHeader {
	next: BlockPtr,
	offset: usize,
}

pub struct Arena {
	inner: UnsafeCell<Inner>,
}

struct Inner {
	head: BlockPtr,
	curr_block: BlockPtr,
	alloc_count: usize,
	last_alloc: NonNull<u8>,
	block_size: usize,
}

impl Arena {
	pub const fn new() -> Self { Self::with_block_size(1024 * 1024) }

	pub const fn with_block_size(block_size: usize) -> Self {
		let head = BlockPtr {
			ptr: NonNull::dangling(),
			size: 0,
		};

		Arena {
			inner: UnsafeCell::new(Inner {
				head,
				curr_block: head,
				alloc_count: 0,
				last_alloc: NonNull::dangling(),
				block_size,
			}),
		}
	}

	unsafe fn reset_all_blocks(&self) {
		let inner = self.inner.get();
		let mut next_block = (*inner).head;
		loop {
			if next_block.size == 0 {
				break;
			}
			let block = next_block.ptr.as_ptr();
			(*block).offset = 0;
			next_block = (*block).next;
		}
	}

	fn block_layout(size: usize) -> Layout {
		unsafe {
			Layout::from_size_align_unchecked(
				std::mem::size_of::<BlockHeader>() + size,
				std::mem::align_of::<BlockHeader>(),
			)
		}
	}

	fn allocate_block(size: usize) -> NonNull<BlockHeader> {
		unsafe {
			let layout = Self::block_layout(size);
			let ptr = std::alloc::alloc(layout);
			if ptr.is_null() {
				handle_alloc_error(layout);
			} else {
				let head: NonNull<BlockHeader> = NonNull::new_unchecked(ptr.cast());
				head.as_ptr().write(BlockHeader {
					next: BlockPtr {
						ptr: NonNull::dangling(),
						size: 0,
					},
					offset: 0,
				});
				head
			}
		}
	}

	fn extend(&self, size: usize) -> NonNull<BlockHeader> {
		let inner = self.inner.get();
		let new = Self::allocate_block(size);
		unsafe {
			let next = BlockPtr { ptr: new, size };
			(*(*inner).curr_block.ptr.as_ptr()).next = next;
			(*inner).curr_block = next;
		}

		new
	}

	fn aligned_offset(&self, align: usize) -> usize {
		unsafe {
			let curr = (*self.inner.get()).curr_block.ptr.as_ptr();
			let base = curr.add(1);
			let unaligned = base.add((*curr).offset) as usize;
			let aligned = (unaligned + align - 1) & !(align - 1);
			aligned - base as usize
		}
	}

	pub fn allocate(&self, layout: Layout) -> NonNull<[u8]> {
		// SAFETY: I said so (and miri agrees).
		unsafe {
			let inner = self.inner.get();

			let ret = if layout.size() > (*inner).block_size {
				// Allocate a dedicated block for this, since it's too big for our current block size.
				NonNull::new_unchecked(std::slice::from_raw_parts_mut(
					self.extend(layout.size()).as_ptr().add(1).cast(),
					layout.size(),
				))
			} else {
				let size = (*inner).curr_block.size;
				if (*inner).curr_block.size == 0 {
					// There's no current block, so create it.
					let new = Self::allocate_block(size);
					let next = BlockPtr { ptr: new, size };
					(*inner).curr_block = next;
				}

				let offset = self.aligned_offset(layout.align());

				let target = if offset + layout.size() > (*inner).curr_block.size {
					// There's not enough space in the current block, so go to the next one.
					let next = (*(*inner).curr_block.ptr.as_ptr()).next;
					if next.size != 0 {
						// There's a next block, so we can use it.
						(*inner).curr_block = next;
					} else {
						// There's no next block, so we need to allocate a new one.
						self.extend((*inner).block_size);
					}

					let offset = self.aligned_offset(layout.align());
					let base: *mut u8 = (*inner).curr_block.ptr.as_ptr().add(1).cast();
					base.add(offset)
				} else {
					let base: *mut u8 = (*inner).curr_block.ptr.as_ptr().add(1).cast();
					// There's enough space in the current block, so use it.
					base.add(offset)
				};

				(*(*inner).curr_block.ptr.as_ptr()).offset += layout.size();
				NonNull::new_unchecked(std::slice::from_raw_parts_mut(target, layout.size()))
			};

			(*inner).alloc_count += 1;
			(*inner).last_alloc = ret.cast();
			ret
		}
	}

	pub unsafe fn deallocate(&self, ptr: NonNull<u8>) {
		let inner = self.inner.get();

		(*inner).alloc_count -= 1;
		if (*inner).alloc_count == 0 {
			self.reset_all_blocks()
		} else if ptr == (*inner).last_alloc {
			let offset = ptr.as_ptr().offset_from((*inner).curr_block.ptr.as_ptr().add(1).cast());
			(*(*inner).curr_block.ptr.as_ptr()).offset = offset as _;
		}
	}
}

impl Drop for Arena {
	fn drop(&mut self) {
		let inner = self.inner.get_mut();
		let mut next_block = inner.head;
		loop {
			if next_block.size == 0 {
				break;
			}
			unsafe {
				let block = next_block.ptr.as_ptr();
				let size = next_block.size;
				next_block = (*block).next;
				let b: *mut u8 = block.cast();
				std::alloc::dealloc(b, Self::block_layout(size));
			}
		}
	}
}

#[cfg(test)]
mod tests {
	use std::intrinsics::copy_nonoverlapping;

	pub use super::*;

	#[test]
	fn allocate() {
		let arena = Arena::new();

		let data_1 = [0u8, 1, 2, 3, 4, 5, 6, 7];
		let ptr_1 = arena.allocate(Layout::from_size_align(data_1.len(), 1).unwrap());
		unsafe { copy_nonoverlapping(data_1.as_ptr(), &mut (*ptr_1.as_ptr())[0], data_1.len()) }

		let data_2 = [8u8, 9, 10, 11, 12, 13, 14, 15];
		let ptr_2 = arena.allocate(Layout::from_size_align(data_2.len(), 1).unwrap());
		unsafe { copy_nonoverlapping(data_2.as_ptr(), &mut (*ptr_2.as_ptr())[0], data_2.len()) }

		unsafe {
			assert_eq!(data_1, ptr_1.as_ref());
			assert_eq!(data_2, ptr_2.as_ref());
		}
	}
}
