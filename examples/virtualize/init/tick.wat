;; A guest whose core start function calls the donut's virtual `tick` while the
;; guest is being instantiated, i.e. after the donut's first fixup phase but
;; before its guest exports are linked. `probe.ticks` reports what it got.
(component
  (import "example:virt/init-host" (instance $host
    (export "tick" (func (result u32)))
    (export "tick-reentrant" (func (result u32)))))
  (alias export $host "tick" (func $f))
  (core func $f-lowered (canon lower (func $f)))
  (core module $M
    (import "host" "f" (func $f (result i32)))
    (global $seen (mut i32) (i32.const 0))
    (func $start (global.set $seen (call $f)))
    (start $start)
    (func (export "ticks") (result i32) (global.get $seen)))
  (core instance $m (instantiate $M (with "host" (instance (export "f" (func $f-lowered))))))
  (func $ticks (result u32) (canon lift (core func $m "ticks")))
  (instance $probe (export "ticks" (func $ticks)))
  (export "example:virt/probe" (instance $probe)))
