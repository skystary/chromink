Pinned upstream: https://github.com/IanManske/quantette
Commit: 0140c6abcc2262766bd2833fadedde1ac3358473 (0.6.0).

The Rust library is statically built for its Oklab Wu initializer and SIMD color conversion.
Wu mathematics are unchanged. Application Lloyd clustering is implemented in trace-core/src/lloyd.rs.
The retired online/mini-batch K-means module, related API branches and random-generator dependencies
are removed, as are upstream test code. Only threads/std are enabled by the application.
The standalone manifest replaces upstream workspace metadata. MIT/Apache-2.0 notices remain intact.
