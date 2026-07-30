use crate::interpreter::execute_code_in_frame;
use crate::machine::{VmHost, VmMachine};
use crate::rt::*;
use crate::space::*;
use crate::value::*;

mod call;
mod frame;

pub use frame::{CallFrame, IntentScopeState};
