# Pre-commit failures and corrections

These came from interactive runs during development, before the first
implementation commit. Their raw output was not written to files; the
excerpts below are copied from the session transcript. Nothing here changed
the library's behaviour after a test had passed.

1. **Strict Clippy, first run on the new tests.** `cargo clippy -p
   limeos-backup-archive --all-targets --locked --offline -- -D warnings`
   failed, shown here filtered to its error and location lines:

   ```text
   error: very complex type used. Consider factoring parts into `type` definitions
      --> crates/backup-archive/src/custody/tests.rs:439:16
   error: very complex type used. Consider factoring parts into `type` definitions
      --> crates/backup-archive/tests/custody.rs:548:16
   ```

   Both test tables now use a local `type Expect = fn(&CustodyError) -> bool;`.

2. **Identity known answers, before the independent cross-check.** The first
   unit-test run used placeholder strings, so
   `identity_encoding_is_versioned_order_sensitive_and_bounded` failed on
   purpose, reporting the Rust value:

   ```text
   assertion `left == right` failed: policy identity encoding v1 changed; bump FORMAT_VERSION and the domain
     left: "5ab96685ef8c9aed9e59792e69dc754f920586c3e08769b5d0be21448b60b556"
    right: "POLICY-KNOWN-ANSWER"
   ```

   The policy value was pinned only after `identity_reference.py`, written from
   the contract's text, computed the same digest and the same 304-byte length.
   The manifest value `85beea48…` was pinned from the Rust run. Its encoding
   is cross-checked separately against four fixture records in
   `identity-reference.txt`.

3. **Defects found by review before any test ran:**
   - `discard` listed a directory another owner had already removed. Listing
     moved after the name check.
   - Forged-record tests built through a JSON `Value` would have been refused
     as non-canonical rather than as inconsistent. They now edit a typed mirror.
   - The duplicate-field case needed a complete duplicate value.
   - A hostile-fixture loop left its source file behind.

   Separately, the fault seam reimplemented the no-replace rename. It now calls
   the production function (commit `9340277`), so the `rename-replaces` mutant
   is detectable.

4. **The first identity-reference evidence file** recorded `reference exit 0`
   from the trailing `sed` in a pipeline rather than from the reference. It
   was regenerated with the reference's own exit status captured, which is
   also 0. Only the regenerated file is kept.
