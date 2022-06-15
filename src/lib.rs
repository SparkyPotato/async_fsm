#![feature(ptr_metadata)]
#![feature(waker_getters)]

use std::{marker::PhantomData, ops::Deref};

use crate::{dyn_future::DynFuture, executor::execute};

mod arena;
mod context;
mod dyn_future;
mod executor;

/// A trait representing a state machine.
pub unsafe trait State: Sized {
	/// The size of the largest future that can be created by `get_future`.
	/// Incorrect size will cause undefined behavior.
	const MAX_FUTURE_SIZE: usize;

	/// The alignment of the largest future that can be created by `get_future`.
	/// Incorrect alignment will cause undefined behavior.
	const MAX_FUTURE_ALIGNMENT: usize;

	/// Data passed to the state machine.
	type Input;

	/// Data yielded when there was no state transition.
	type Yield;

	fn get_future(&self, args: Input<Self::Input, Self::Yield>, into: &mut DynFuture<Self>);
}

pub enum PollResult<'a, S: State> {
	Blocked,
	Yielded(S::Yield),
	Transition(&'a S),
}

pub struct Fsm<'a, S: State> {
	state: S,
	future: DynFuture<'a, S>,
}

impl<S: State> Fsm<'_, S> {
	pub fn new(state: S) -> Self {
		let mut future = DynFuture::new(S::MAX_FUTURE_SIZE, S::MAX_FUTURE_ALIGNMENT);
		state.get_future(Input { _phantom: PhantomData }, &mut future);

		Self { future, state }
	}

	/// Synchronously runs the state machine until either:
	/// * A transition occurs.
	/// * A value is yielded.
	/// * The state machine is blocked on another future.
	pub fn poll(&mut self, input: S::Input) -> PollResult<S> {
		let res = execute(self.future.get(), &mut self.state, input);
		if let PollResult::Transition(state) = res {
			state.get_future(Input { _phantom: PhantomData }, &mut self.future)
		}
		res
	}

	/// Set the current state, discarding the previous state.
	pub fn set_state(&mut self, state: S) {
		self.state = state;
		self.state.get_future(Input { _phantom: PhantomData }, &mut self.future);
	}
}

impl<S: State> Deref for Fsm<'_, S> {
	type Target = S;

	fn deref(&self) -> &Self::Target { &self.state }
}

pub struct Input<T, Y> {
	_phantom: PhantomData<(T, Y)>,
}

impl<T, Y> Copy for Input<T, Y> {}

impl<T, Y> Clone for Input<T, Y> {
	fn clone(&self) -> Self { *self }
}

pub struct Yield<Y> {
	value: Option<Y>,
}

pub const fn size_of_impl_return<A, R, F: FnOnce(A) -> R>(_: &F) -> usize { std::mem::size_of::<R>() }

pub const fn align_of_impl_return<A, R, F: FnOnce(A) -> R>(_: &F) -> usize { std::mem::align_of::<R>() }

#[doc(hidden)]
pub const fn const_max(a: usize, b: usize) -> usize {
	if a > b {
		a
	} else {
		b
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

	unsafe impl State for PingPong {
		type Input = bool;
		type Yield = ();

		const MAX_FUTURE_ALIGNMENT: usize = const_max(align_of_impl_return(&ping), align_of_impl_return(&pong));
		const MAX_FUTURE_SIZE: usize = const_max(size_of_impl_return(&ping), size_of_impl_return(&pong));

		fn get_future(&self, input: Input<Self::Input, Self::Yield>, into: &mut DynFuture<Self>) {
			unsafe {
				match self {
					PingPong::Ping => into.write(ping(input)),
					PingPong::Pong => into.write(pong(input)),
				}
			}
		}
	}

	async fn ping(mut input: Input<bool, ()>) -> PingPong {
		loop {
			let x = input.get();
			if x {
				return PingPong::Pong;
			} else {
				input.yield_(()).await;
			}
		}
	}

	async fn pong(mut input: Input<bool, ()>) -> PingPong {
		loop {
			let x = input.get();
			if x {
				return PingPong::Ping;
			} else {
				input.yield_(()).await;
			}
		}
	}

	#[test]
	fn test() {
		let mut fsm = Fsm::new(PingPong::Ping);
		assert!(matches!(fsm.poll(true), PollResult::Transition(PingPong::Pong)));
		assert_eq!(*fsm, PingPong::Pong);
		assert!(matches!(fsm.poll(false), PollResult::Yielded(())));
		assert_eq!(*fsm, PingPong::Pong);
		assert!(matches!(fsm.poll(true), PollResult::Transition(PingPong::Ping)));
		assert_eq!(*fsm, PingPong::Ping);
	}
}
