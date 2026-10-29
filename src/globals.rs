//! `system_suite::foundation()`: the running system's `Foundation`,
//! published through global-lib (feature `globals`).

use global_lib::Global;

use crate::Foundation;

static FOUNDATION: Global<Foundation> = Global::new();

/// The running system's [`Foundation`], for code that cannot be handed one
/// (a plugin loaded by name, a callback registered by a third-party
/// library).
///
/// It is published when the system reaches `Ready` and withdrawn when its
/// shutdown starts; it is `None` before, after, and for a system built
/// with [`process_wide(false)`](crate::Builder::process_wide) or with
/// [`Member::Globals`](crate::Member::Globals) disabled. Prefer passing
/// `Foundation` explicitly; this exists for the cases where that is
/// impossible.
///
/// # Examples
///
/// ```
/// assert!(system_suite::foundation().is_none()); // no system running here
/// ```
#[must_use]
pub fn foundation() -> Option<Foundation> {
    FOUNDATION.value()
}

/// Publishes `foundation`. Called at boot step 17.
pub(crate) fn publish(foundation: &Foundation) {
    let _previous = FOUNDATION.set(foundation.clone());
}

/// Withdraws the published foundation. Called at shutdown step 1.
pub(crate) fn withdraw() {
    let _taken = FOUNDATION.take();
}
