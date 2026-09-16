//! Specialized checking engines and CLI integration helpers.
//! Dependency direction: adapters -> core; the core never imports this crate.
pub mod audit;
pub mod checkers;
pub mod init;
pub mod selftest;
pub mod severity;
