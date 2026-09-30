//! CSV sensor log: the bounded queue between the sampler and the writer
//! thread, the writer itself and the filesystem it writes through.

// Nothing calls the log module yet; Task 5 wires it into the app and removes
// this allow.
#![cfg_attr(not(test), allow(dead_code))]

pub mod fs;
pub mod queue;
pub mod writer;
