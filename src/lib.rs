#![allow(incomplete_features)]
#![feature(generic_const_exprs)]
#![feature(specialization)]
#![feature(ptr_metadata)]
#![feature(waker_getters)]

use std::{
	alloc::Layout,
	future::Future,
	marker::{PhantomData, PhantomPinned},
	mem::MaybeUninit,
	ops::Deref,
	pin::Pin,
	ptr::{DynMetadata, NonNull},
	task::{Context, Poll, RawWaker, RawWakerVTable, Waker},
};

use elain::{Align, Alignment};

struct StackOrHeapBuffer<const SIZE: usize, const ALIGN: usize, const ON_STACK: bool>
where
	Align<{ align_for_buffer(SIZE, ON_STACK) }>: Alignment,
	[(); size_for_buffer(SIZE, ON_STACK)]:,
	[(); 1 - ON_STACK as usize]:,
{
	_align: Align<{ align_for_buffer(SIZE, ON_STACK) }>,
	buffer: [MaybeUninit<u8>; size_for_buffer(SIZE, ON_STACK)],
	ptr: [NonNull<u8>; 1 - ON_STACK as usize],
}

impl<const SIZE: usize, const ALIGN: usize, const ON_STACK: bool> StackOrHeapBuffer<SIZE, ALIGN, ON_STACK>
where
	Align<{ align_for_buffer(SIZE, ON_STACK) }>: Alignment,
	[(); size_for_buffer(SIZE, ON_STACK)]:,
	[(); 1 - ON_STACK as usize]:,
{
	fn new() -> Self {
		if ON_STACK {
			Self {
				_align: Align::NEW,
				buffer: [MaybeUninit::uninit(); size_for_buffer(SIZE, ON_STACK)],
				ptr: [NonNull::dangling(); 1 - ON_STACK as usize],
			}
		} else {
			unsafe {
				let ptr = std::alloc::alloc(Layout::from_size_align(SIZE, ALIGN).unwrap());

				Self {
					_align: Align::NEW,
					buffer: [MaybeUninit::uninit(); size_for_buffer(SIZE, ON_STACK)],
					ptr: [NonNull::new_unchecked(ptr); 1 - ON_STACK as usize],
				}
			}
		}
	}

	fn as_ptr(&self) -> *mut u8 {
		if ON_STACK {
			self.buffer.as_ptr() as *const _ as _
		} else {
			self.ptr[0].as_ptr()
		}
	}
}

impl<const SIZE: usize, const ALIGN: usize, const ON_STACK: bool> Drop for StackOrHeapBuffer<SIZE, ALIGN, ON_STACK>
where
	Align<{ align_for_buffer(SIZE, ON_STACK) }>: Alignment,
	[(); size_for_buffer(SIZE, ON_STACK)]:,
	[(); 1 - ON_STACK as usize]:,
{
	fn drop(&mut self) {
		if !ON_STACK {
			unsafe {
				std::alloc::dealloc(self.ptr[0].as_ptr(), Layout::from_size_align(SIZE, ALIGN).unwrap());
			}
		}
	}
}

#[repr(C)]
pub struct DynFuture<O, const SIZE: usize, const ALIGN: usize, const UNPIN: bool>
where
	Align<{ align_for_buffer(SIZE, UNPIN) }>: Alignment,
	[(); size_for_buffer(SIZE, UNPIN)]:,
	[(); 1 - UNPIN as usize]:,
{
	_phantom: PhantomPinned,
	buf: StackOrHeapBuffer<SIZE, ALIGN, UNPIN>,
	metadata: Option<DynMetadata<dyn Future<Output = O>>>,
}

impl<O, const SIZE: usize, const ALIGN: usize, const UNPIN: bool> DynFuture<O, SIZE, ALIGN, UNPIN>
where
	Align<{ align_for_buffer(SIZE, UNPIN) }>: Alignment,
	[(); size_for_buffer(SIZE, UNPIN)]:,
	[(); 1 - UNPIN as usize]:,
{
	pub unsafe fn new() -> Self {
		Self {
			_phantom: PhantomPinned,
			buf: StackOrHeapBuffer::new(),
			metadata: None,
		}
	}
}

impl<O, const SIZE: usize, const ALIGN: usize> DynFuture<O, SIZE, ALIGN, false>
where
	Align<{ align_for_buffer(SIZE, false) }>: Alignment,
	[(); size_for_buffer(SIZE, false)]:,
{
	pub fn write(&mut self, future: impl Future<Output = O> + 'static) {
		unsafe {
			if let Some(metadata) = self
				.metadata
				.replace(std::ptr::metadata(&future as &dyn Future<Output = O>))
			{
				let ptr: *mut dyn Future<Output = O> = std::ptr::from_raw_parts_mut(self.buf.as_ptr() as _, metadata);
				std::ptr::drop_in_place(ptr);
			}

			assert!(std::mem::size_of_val(&future) <= SIZE, "Future is too large");
			assert!(
				std::mem::align_of_val(&future) <= ALIGN,
				"Future alignment is too large"
			);
			std::ptr::copy_nonoverlapping(&future as *const _, self.buf.as_ptr() as _, 1);
		}
	}
}

impl<O, const SIZE: usize, const ALIGN: usize> DynFuture<O, SIZE, ALIGN, true>
where
	Align<{ align_for_buffer(SIZE, true) }>: Alignment,
	[(); size_for_buffer(SIZE, true)]:,
{
	pub fn write(&mut self, future: impl Future<Output = O> + Unpin + 'static) {
		unsafe {
			if let Some(metadata) = self
				.metadata
				.replace(std::ptr::metadata(&future as &dyn Future<Output = O>))
			{
				let ptr: *mut dyn Future<Output = O> = std::ptr::from_raw_parts_mut(self.buf.as_ptr() as _, metadata);
				std::ptr::drop_in_place(ptr);
			}

			assert!(std::mem::size_of_val(&future) <= SIZE, "Future is too large");
			std::ptr::copy_nonoverlapping(
				&future as *const _ as *const _,
				self.buf.as_ptr() as _,
				std::mem::size_of_val(&future),
			);
		}
	}
}

impl<O, const SIZE: usize, const ALIGN: usize, const UNPIN: bool> Future for DynFuture<O, SIZE, ALIGN, UNPIN>
where
	Align<{ align_for_buffer(SIZE, UNPIN) }>: Alignment,
	[(); size_for_buffer(SIZE, UNPIN)]:,
	[(); 1 - UNPIN as usize]:,
{
	type Output = O;

	fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
		unsafe {
			let ptr: *mut dyn Future<Output = O> =
				std::ptr::from_raw_parts_mut(self.buf.as_ptr() as _, self.metadata.unwrap());
			Pin::new_unchecked(&mut *ptr).poll(cx)
		}
	}
}

impl<O, const SIZE: usize, const ALIGN: usize, const UNPIN: bool> Drop for DynFuture<O, SIZE, ALIGN, UNPIN>
where
	Align<{ align_for_buffer(SIZE, UNPIN) }>: Alignment,
	[(); size_for_buffer(SIZE, UNPIN)]:,
	[(); 1 - UNPIN as usize]:,
{
	fn drop(&mut self) {
		unsafe {
			let ptr: *mut dyn Future<Output = O> =
				std::ptr::from_raw_parts_mut(self.buf.as_ptr() as _, self.metadata.unwrap());
			std::ptr::drop_in_place(ptr);
		}
	}
}

/// A trait representing a state machine.
pub trait State: Sized {
	/// The size of the largest future that can be created by `get_future`.
	const MAX_FUTURE_SIZE: usize;

	/// The alignment of the largest future that can be created by `get_future`.
	const MAX_FUTURE_ALIGNMENT: usize;

	/// If all the futures returned by `get_future` are `Unpin`.
	const ALL_UNPIN: bool;

	type Input;

	fn get_future(
		&self, args: Input<Self::Input>,
		into: &mut DynFuture<Self, { Self::MAX_FUTURE_SIZE }, { Self::MAX_FUTURE_ALIGNMENT }, { Self::ALL_UNPIN }>,
	) where
		Align<{ align_for_buffer(Self::MAX_FUTURE_SIZE, Self::ALL_UNPIN) }>: Alignment,
		[(); size_for_buffer(Self::MAX_FUTURE_SIZE, Self::ALL_UNPIN)]:,
		[(); 1 - Self::ALL_UNPIN as usize]:;
}

pub struct Fsm<S: State>
where
	Align<{ align_for_buffer(S::MAX_FUTURE_SIZE, S::ALL_UNPIN) }>: Alignment,
	[(); size_for_buffer(S::MAX_FUTURE_SIZE, S::ALL_UNPIN)]:,
	[(); S::MAX_FUTURE_ALIGNMENT]:,
	[(); 1 - S::ALL_UNPIN as usize]:,
{
	state: S,
	future: DynFuture<S, { S::MAX_FUTURE_SIZE }, { S::MAX_FUTURE_ALIGNMENT }, { S::ALL_UNPIN }>,
}

impl<S: State> Fsm<S>
where
	Align<{ align_for_buffer(S::MAX_FUTURE_SIZE, S::ALL_UNPIN) }>: Alignment,
	[(); size_for_buffer(S::MAX_FUTURE_SIZE, S::ALL_UNPIN)]:,
	[(); S::MAX_FUTURE_ALIGNMENT]:,
	[(); 1 - S::ALL_UNPIN as usize]:,
{
	pub fn new(state: S) -> Self {
		let mut future = unsafe { DynFuture::new() };
		state.get_future(Input { _phantom: PhantomData }, &mut future);

		Self { future, state }
	}

	/// Runs the state machine until either a transition is triggered, or a state suspends itself.
	///
	/// Returns if a transition was triggered.
	pub fn poll(&mut self, input: S::Input) -> bool {
		unsafe {
			static VTABLE: RawWakerVTable = RawWakerVTable::new(clone, |_| {}, |_| {}, |_| {});
			fn clone(data: *const ()) -> RawWaker { RawWaker::new(data, &VTABLE) }

			let pin = Pin::new_unchecked(&mut self.future);
			let mut ctx = FsmContext { data: Some(input) };

			let value = pin.poll(&mut Context::from_waker(&Waker::from_raw(clone(
				&mut ctx as *mut _ as *const _ as _,
			))));

			match value {
				Poll::Ready(x) => {
					x.get_future(Input { _phantom: PhantomData }, &mut self.future);
					self.state = x;
					true
				},
				Poll::Pending => false,
			}
		}
	}
}

impl<S: State> Deref for Fsm<S>
where
	Align<{ align_for_buffer(S::MAX_FUTURE_SIZE, S::ALL_UNPIN) }>: Alignment,
	[(); size_for_buffer(S::MAX_FUTURE_SIZE, S::ALL_UNPIN)]:,
	[(); S::MAX_FUTURE_ALIGNMENT]:,
	[(); 1 - S::ALL_UNPIN as usize]:,
{
	type Target = S;

	fn deref(&self) -> &Self::Target { &self.state }
}

struct FsmContext<T> {
	data: Option<T>,
}

pub struct Input<T> {
	_phantom: PhantomData<T>,
}

impl<T> Unpin for Input<T> {}

impl<T> Input<T> {
	pub fn get(&mut self) -> Pin<&mut Self> { Pin::new(self) }
}

impl<T> Copy for Input<T> {}

impl<T> Clone for Input<T> {
	fn clone(&self) -> Self { *self }
}

impl<T> Future for Input<T> {
	type Output = T;

	fn poll(self: Pin<&mut Self>, ctx: &mut Context<'_>) -> Poll<Self::Output> {
		unsafe {
			let data = ctx.waker().as_raw().data() as *const FsmContext<T> as *mut FsmContext<T>;
			match (*data).data.take() {
				Some(x) => Poll::Ready(x),
				None => Poll::Pending,
			}
		}
	}
}

pub const fn size_of_impl_return<A, R, F: FnOnce(A) -> R>(_: &F) -> usize { std::mem::size_of::<R>() }

pub const fn align_of_impl_return<A, R, F: FnOnce(A) -> R>(_: &F) -> usize { std::mem::align_of::<R>() }

pub const fn is_impl_return_unpin<A, R, F: FnOnce(A) -> R>(_: &F) -> bool { <R as IsUnpin>::IS_UNPIN }

trait IsUnpin {
	const IS_UNPIN: bool;
}

impl<T> IsUnpin for T {
	default const IS_UNPIN: bool = false;
}

impl<T: Unpin> IsUnpin for T {
	const IS_UNPIN: bool = true;
}

#[doc(hidden)]
const fn max_size(a: usize, b: usize) -> usize {
	if a > b {
		a
	} else {
		b
	}
}

#[doc(hidden)]
pub const fn size_for_buffer(size: usize, on_stack: bool) -> usize {
	if on_stack {
		size
	} else {
		0
	}
}

#[doc(hidden)]
pub const fn align_for_buffer(align: usize, on_stack: bool) -> usize {
	if on_stack {
		align
	} else {
		1
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[derive(Debug, Eq, PartialEq)]
	enum PingPong {
		Ping,
		Pong,
	}

	impl State for PingPong {
		type Input = bool;

		const ALL_UNPIN: bool = is_impl_return_unpin(&ping) && is_impl_return_unpin(&pong);
		const MAX_FUTURE_ALIGNMENT: usize = max_size(align_of_impl_return(&ping), align_of_impl_return(&pong));
		const MAX_FUTURE_SIZE: usize = max_size(size_of_impl_return(&ping), size_of_impl_return(&pong));

		fn get_future(
			&self, input: Input<Self::Input>,
			into: &mut DynFuture<Self, { Self::MAX_FUTURE_SIZE }, { Self::MAX_FUTURE_ALIGNMENT }, { Self::ALL_UNPIN }>,
		) where
			Align<{ align_for_buffer(Self::MAX_FUTURE_SIZE, Self::ALL_UNPIN) }>: Alignment,
			[(); size_for_buffer(Self::MAX_FUTURE_SIZE, Self::ALL_UNPIN)]:,
			[(); 1 - Self::ALL_UNPIN as usize]:,
		{
			match self {
				PingPong::Ping => DynFuture::<
					Self,
					{ Self::MAX_FUTURE_SIZE },
					{ Self::MAX_FUTURE_ALIGNMENT },
					{ Self::ALL_UNPIN },
				>::write(into, ping(input)),
				PingPong::Pong => DynFuture::<
					Self,
					{ Self::MAX_FUTURE_SIZE },
					{ Self::MAX_FUTURE_ALIGNMENT },
					{ Self::ALL_UNPIN },
				>::write(into, pong(input)),
			}
		}
	}

	async fn ping(mut input: Input<bool>) -> PingPong {
		loop {
			let x = input.get().await;
			if x {
				return PingPong::Pong;
			}
		}
	}

	async fn pong(mut input: Input<bool>) -> PingPong {
		loop {
			let x = input.get().await;
			if x {
				return PingPong::Ping;
			}
		}
	}

	#[test]
	fn test() {
		assert_eq!(PingPong::MAX_FUTURE_SIZE, 16);
		assert_eq!(PingPong::MAX_FUTURE_ALIGNMENT, 8);
		assert_eq!(PingPong::ALL_UNPIN, false);

		let mut fsm = Fsm::new(PingPong::Ping);
		assert!(fsm.poll(true));
		assert_eq!(*fsm, PingPong::Pong);
		assert!(!fsm.poll(false));
		assert_eq!(*fsm, PingPong::Pong);
		assert!(fsm.poll(true));
		assert_eq!(*fsm, PingPong::Ping);
	}
}
