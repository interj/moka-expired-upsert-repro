# moka expired-upsert repro

```
$ cargo run
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.02s
     Running `target/debug/moka-expired-upsert-repro`
listener: "first" Expired -> releasing device 1
listener: "second" Expired -> releasing device 2
listener: "third" Expired -> releasing device 3

wrote straight over it  : cache = Some("successor"), device = None
retired it first        : cache = Some("successor"), device = Some("successor")
never replaced          : cache = None, device = None

Key 1 is live in its cache with no device: the expired value's cleanup released the
successor's resource, and only session setup installs one, so nothing restores it.
```
