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

Cloud Android testing completed an unshaped Gutenberg EPUB download, but fresh
connections under256kbps shaping repeatedly hit the8-second connection budget.
File-transfer clients now allow25seconds for connection establishment; the
one-byte capability probe allows30seconds overall and retries transient network
errors with the configured bound. Protocol/version errors still stop immediately.
Real-socket regressions first failed for an interrupted initial probe and for
pause while a probe had headers but no body; both paths are now covered.
A further encoded-response regression failed with three probe requests instead
of one. Restricting retries to classified transport/timeouts resolves it while
retaining immediate rejection of encoding and redirect protocol violations.

Further Android controls showed that a longer connection budget and HTTP/1.1
did not resolve Emulator37's traditional256kbps path: the independent Android
platform HTTPS client also received0bytes and an unexpected EOF. The final CI
uses a loopback TCP tunnel with a shared32,000B/s downstream budget and preserves
the APK's DNS and end-to-end TLS. This is a different controlled network path,
not a claim that the traditional limiter now works. Exact sources and evidence
are recorded in `docs/android-shaping-reference.md`.

CI review also closed empty verify-only runs for x86, optional diagnostics
preventing the actual app test, tunnel half-close truncation, invisible-node
selection, the sixth chapter boundary and transient snapshot errors killing the
test bridge. The half-close regression first failed and then passed; two flows
totaling65,536bytes share the rate budget without changing bytes. The live accessibility bridge
clears cached nodes before each snapshot so old button coordinates cannot guide
touches after layout changes. Both review axes accepted these fixes.

The final signed x86_64 artifact passed the entire actual Android flow in
run38892027698, including background progress, pause, process death, real long
touch, continuation and actual EPUB paragraphs. This runtime acceptance reuses
the independently verified signed artifact from the successful three-package
job38858699264. Application sources are unchanged in subsequent CI/docs commits.

The universal artifact then passed the same complete flow in run38892023366,
including221,184rounded bytes retained across process death and actual reader
content. Its initial attempt was blocked before Python by a system Quickstep
ANR and was rerun without changing the APK or weakening product checks. The x86
artifact is statically verified, without a32-bit runtime claim.
