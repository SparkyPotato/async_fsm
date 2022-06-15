use std::{
	future::Future,
	pin::Pin,
	task::{Context, Poll, RawWaker, RawWakerVTable, Waker},
};

use crate::{
	context::{get_input_ptr, set_input_ptr},
	Input,
	PollResult,
	State,
	Yield,
};

static VTABLE: RawWakerVTable = RawWakerVTable::new(clone, |_| {}, |_| {}, |_| {});

fn clone(data: *const ()) -> RawWaker { RawWaker::new(data, &VTABLE) }

pub fn execute<'a, S: State>(
	future: Pin<&mut dyn Future<Output = S>>, store_state: &'a mut S, input: S::Input,
) -> PollResult<'a, S> {
	let mut input = Some(input);
	let input_ptr = &mut input as *mut _ as *mut ();
	set_input_ptr(input_ptr);
	let mut yield_result: Option<S::Yield> = None;
	let yield_ptr = &mut yield_result as *mut _ as *mut ();

	let waker = clone(yield_ptr);
	match future.poll(&mut Context::from_waker(&unsafe { Waker::from_raw(waker) })) {
		Poll::Ready(state) => {
			*store_state = state;
			PollResult::Transition(store_state)
		},
		Poll::Pending => match yield_result {
			Some(yield_result) => PollResult::Yielded(yield_result),
			None => PollResult::Blocked,
		},
	}
}

impl<T, Y> Input<T, Y> {
	/// Yield a value to the caller.
	pub fn yield_(&mut self, value: Y) -> Yield<Y> { Yield { value: Some(value) } }

	pub fn get(&self) -> T {
		let ptr = get_input_ptr();
		if !ptr.is_null() {
			// SAFETY: input_ptr is always set to a valid `Option<T>` instance.
			unsafe {
				let ptr = ptr as *mut Option<T>;
				ptr.read().take().expect("Input::get() already called")
			}
		} else {
			panic!("Unknown error");
		}
	}
}

impl<Y: Unpin> Future for Yield<Y> {
	type Output = ();

	fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
		if let Some(value) = self.value.take() {
			// SAFETY: We're only created inside our executor, so data is what we expect.
			unsafe {
				let yield_ptr = cx.waker().as_raw().data() as *mut () as *mut Option<Y>;
				*yield_ptr = Some(value);
			}
			Poll::Pending
		} else {
			Poll::Ready(())
		}
	}
}

impl<Y: Unpin> Unpin for Yield<Y> {}
