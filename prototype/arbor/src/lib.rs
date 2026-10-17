//! The first Arbor prototype: a rootless layer on today's Linux. This version has worlds: instant copy-on-write
//! copies of a folder, changed by commands run inside them, then kept or thrown away whole; sealed cells with limits
//! on memory, CPU, processes and time, and a filter on the kernel calls they make; keys that open more than the
//! world until a set time, including the network to the hosts they name; the outbox, where what a command in a
//! world asks to do outside it waits for a person, who releases it to run once or refuses it, within a key's budget;
//! and the ledger, a record of every step and what it cost.

pub mod cell;
pub mod cgroup;
pub mod error;
pub mod key;
pub mod ledger;
pub mod net;
pub mod outbox;
pub mod paths;
pub mod seccomp;
pub mod sys;
pub mod table;
pub mod tree;
pub mod world;
