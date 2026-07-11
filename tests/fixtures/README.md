# Historical `roaring` snapshots

`roaring-0.11.4-mixed.hex` is an immutable, textual RLE representation of an 8,261-byte standard
Roaring bitmap snapshot. It was generated once with `roaring =0.11.4` using `serialize_into`, then
checked in with its SHA-256 and semantic contents recorded in the fixture itself. Tests decode the
fixed bytes and place them behind the current stable header; they never call the current
`roaring` writer to create the fixture.

The fixture contains array, bitmap-density, and run containers under five high-16-bit keys. It
therefore uses the standard-format offset table as well as all container kinds used by this crate.
It is a reader-compatibility gate: an upstream `roaring` update is acceptable only if the current
reader still opens these historical bytes with the documented semantics. Writer byte identity is
not required because a standard Roaring encoder may select a different, still compatible form.
