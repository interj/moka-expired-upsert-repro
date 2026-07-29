//! moka 0.12.15: writing to a key whose entry has expired but has not been purged yet is reported
//! to the eviction listener as that entry *expiring*, while the key already holds the value just
//! written. A listener that releases a per-key resource on expiry therefore destroys the resource
//! of the session that replaced it, and `RemovalCause` gives it no way to tell the two apart.
//!
//! An expired entry is only removed by housekeeping, and a write is not housekeeping: `insert`
//! notifies the listener from `do_insert_with_hash` before it ever calls `apply_writes_if_needed`.
//! So no amount of elapsed time lets the purge get there first.
//!
//! One cache per scenario, because the first write after expiry runs housekeeping, which would
//! purge the other scenarios' parked entries.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use moka::future::Cache;

const TTI: Duration = Duration::from_millis(100);

/// A resource installed per key when a session is created, and released when that key goes away:
/// a device entry, a firewall rule, a leased address. Only session setup ever installs one.
type Devices = Arc<Mutex<HashMap<u32, &'static str>>>;

fn cache_releasing_devices(devices: &Devices) -> Cache<u32, &'static str> {
    let devices = Arc::clone(devices);
    Cache::builder()
        .time_to_idle(TTI)
        .eviction_listener(move |key, name, cause| {
            // The listener has only the cause to go on, so it releases the key's resource.
            println!("listener: {name:?} {cause:?} -> releasing device {key}");
            devices.lock().unwrap().remove(&key);
        })
        .build()
}

async fn create_session(cache: &Cache<u32, &'static str>, devices: &Devices, key: u32, name: &'static str) {
    devices.lock().unwrap().insert(key, name);
    cache.insert(key, name).await;
}

#[tokio::main]
async fn main() {
    let devices: Devices = Arc::default();
    let wrote_over = cache_releasing_devices(&devices);
    let retired_first = cache_releasing_devices(&devices);
    let never_replaced = cache_releasing_devices(&devices);

    create_session(&wrote_over, &devices, 1, "first").await;
    create_session(&retired_first, &devices, 2, "second").await;
    create_session(&never_replaced, &devices, 3, "third").await;

    // Long past the time to idle. Nothing reads and nothing runs housekeeping, so all three
    // entries are expired and still in their maps.
    tokio::time::sleep(TTI * 5).await;

    // A new session under the same key, written straight over the expired one.
    create_session(&wrote_over, &devices, 1, "successor").await;

    // The fix, entirely caller-side: retire the expired entry first. `invalidate` removes it
    // itself and awaits the listener before returning, so teardown completes before setup starts.
    retired_first.invalidate(&2).await;
    assert!(devices.lock().unwrap().get(&2).is_none(), "invalidate should have run the listener");
    create_session(&retired_first, &devices, 2, "successor").await;

    // And a key that really did go away, to show the same listener is right when it is told so.
    never_replaced.run_pending_tasks().await;

    let devices = devices.lock().unwrap();
    println!();
    for (key, cache, label) in [
        (1, &wrote_over, "wrote straight over it"),
        (2, &retired_first, "retired it first"),
        (3, &never_replaced, "never replaced"),
    ] {
        println!("{label:24}: cache = {:?}, device = {:?}", cache.get(&key).await, devices.get(&key));
    }

    assert!(
        wrote_over.get(&1).await.is_some() && devices.get(&1).is_none(),
        "expected the broken state this repro is about"
    );
    assert!(
        retired_first.get(&2).await.is_some() && devices.get(&2).is_some(),
        "retiring the expired entry first keeps the successor whole"
    );
    assert!(
        never_replaced.get(&3).await.is_none() && devices.get(&3).is_none(),
        "a key that really expired is consistent"
    );
    println!(
        "\nKey 1 is live in its cache with no device: the expired value's cleanup released the\n\
         successor's resource, and only session setup installs one, so nothing restores it."
    );
}
