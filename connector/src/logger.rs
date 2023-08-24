pub use log;
use std::cell::RefCell;

// TODO LOG_TARGET should not be thread-local
thread_local! {
    // Initialize at beginning of running case
    pub static LOG_TARGET: RefCell<String> = RefCell::new(String::new());
}

#[macro_export(local_inner_macros)]
macro_rules! trace {
    ($( $args:tt )*) => {
        $crate::logger::LOG_TARGET.with(|c| {
            if !c.borrow().is_empty() {
                $crate::logger::log::trace!(target: &c.borrow(), $( $args )*);
            } else {
                $crate::logger::log::trace!($( $args )*);
            }
        });
    }
}

#[macro_export(local_inner_macros)]
macro_rules! debug {
    ($( $args:tt )*) => {
        $crate::logger::LOG_TARGET.with(|c| {
            if !c.borrow().is_empty() {
                $crate::logger::log::debug!(target: &c.borrow(), $( $args )*);
            } else {
                $crate::logger::log::debug!($( $args )*);
            }
        });
    }
}

#[macro_export(local_inner_macros)]
macro_rules! info {
    ($( $args:tt )*) => {
        $crate::logger::LOG_TARGET.with(|c| {
            if !c.borrow().is_empty() {
                $crate::logger::log::info!(target: &c.borrow(), $( $args )*);
            } else {
                $crate::logger::log::info!($( $args )*);
            }
        });
    }
}

#[macro_export(local_inner_macros)]
macro_rules! warn {
    ($( $args:tt )*) => {
        $crate::logger::LOG_TARGET.with(|c| {
            if !c.borrow().is_empty() {
                $crate::logger::log::warn!(target: &c.borrow(), $( $args )*);
            } else {
                $crate::logger::log::warn!($( $args )*);
            }
        });
    }
}

#[macro_export(local_inner_macros)]
macro_rules! error {
    ($( $args:tt )*) => {
        $crate::logger::LOG_TARGET.with(|c| {
            if !c.borrow().is_empty() {
                ::std::eprintln!($( $args )*);
                $crate::logger::log::error!(target: &c.borrow(), $( $args )*);
            } else {
                ::std::eprintln!($( $args )*);
                $crate::logger::log::error!($( $args )*);
            }
        })
    }
}
