# 0.8 review against 677086d

Scope: the user's download interruption, continue/retry, task long press,
multiconnection and DoH complaints, plus signed GitHub Actions delivery.
The two code-review axes reviewed the working implementation independently.

## Standards

No documented boundary violations found. Three correctness findings:

- P1: response headers could stall indefinitely before the body idle timeout.
  Fixed with a separate25-second response deadline, including each redirect hop;
  the stalled-header regression uses a real accepting-but-silent HTTP server.
- P1: a fast DNS address family could exclude slower usable addresses for the
  cache TTL. Fixed by keeping the other query alive and merging its positive-TTL
  addresses, with an epoch preventing an old request repopulating cleared caches.
  Both delayed A and delayed AAAA are covered by TLS/wire regressions.
- P2: overlapping cancel/resume could mark an active worker cancelled. Pause and
  cancel now hold the same task operation guard as remove/restart until workers
  finish checkpointing; resume/enqueue reject changes while that guard is held.

## Spec

Three correctness findings, no unrelated expansion identified:

- P1: the response-header stall violated recoverable download behavior. Fixed
  as above, without imposing a whole-file deadline on slow large downloads.
- P2: a completed file whose shelf entry was removed could not be read/managed
  after enqueue returned a stale library ID. The manager validates and reattaches
  the saved file; a regression proves this needs no new network acquisition.
- P2: a fast-family DoH cache could drop the only usable slower family. Fixed
  with background completion and bounded TTL-aware cache merging as above.

Three findings on each axis were addressed. The highest severity on each axis
was P1 (unbounded response-header wait and DNS address-family availability).

Live hidden-native testing then reproduced an IPv6-only initial DNS answer on
a Windows network without an IPv6 route. The resolver now checks local route
availability without transmitting UDP packets before returning just one family;
if that family has no route, it waits for the other query. Late answers still
merge into the cache. The Android smoke selects a large Moby Dick EPUB through
search, instead of depending on the first changing homepage recommendation.

A deterministic FIFO-lock regression then exposed a completion race in pause-all:
the task ID snapshot and status updates used separate locks, allowing completion
between them to be overwritten as Pausing/Paused. The test failed with Paused
instead of Completed. Both operations now use one lock, preserving completion
and avoiding an unnecessary later redownload of a valid saved file.
