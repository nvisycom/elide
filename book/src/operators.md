# Operators

An operator produces the replacement for one entity. They live in
`elide::redaction::operators`, also reachable as
`elide::prelude::operators`.

## Text

For `dana.reed@example.com`:

| Operator | Result |
|---|---|
| `Keep` | Unchanged, but recorded as reviewed |
| `Erase` | Removed |
| `Replace::new("[EMAIL]")` | `[EMAIL]` |
| `Mask::stars()` | `*********************` |
| `Mask::stars().with_keep_suffix(11)` | `**********example.com` |
| `Truncate` | Shortened to a kept prefix or suffix, with no placeholder |
| `Pseudonymize` | The same surrogate every time this entity appears |
| `Sha2Hash` | A SHA-256 or SHA-512 digest (feature `operator-sha2`) |
| `HmacHash` | A keyed digest (feature `operator-hmac`) |
| `AesEncrypt` | Ciphertext that can be decrypted later (feature `operator-aes`) |
| `Fake` | A realistic stand-in, such as another email (feature `operator-fake`) |

For numbers and dates:

| Operator | Result |
|---|---|
| `Clamp` | Values outside a range collapsed into a bucket, such as `90+` for an age |
| `GeneralizeDate` | A coarser date, such as the year alone (feature `operator-datetime`) |

`WithFallback` chains two operators: if the first cannot handle a value,
such as `GeneralizeDate` on text that is not a date, the second runs.

## Images, audio, and tables

| Modality | Operators |
|---|---|
| `Image` | `Blur::new(sigma)`, `Pixelate::new(block_size)`, `Blackbox::default()` |
| `Audio` | `Silence`, `Beep::new(hz)` |
| `Tabular` | `DropRow`, `DropColumn`, and the text operators on a cell |

`Erase` and `Keep` work in every modality.

## Leak profiles

Every operator declares a `LeakProfile`, how much of the original its output
still exposes:

- **Irrecoverable**: nothing survives. `Erase`, `Replace`, `Blackbox`.
- **Partial**: some survives by design. `Mask` with a kept suffix,
  `Truncate`, `GeneralizeDate`.
- **Recoverable**: whoever holds the key or the vault can restore the
  original. `AesEncrypt`, `Pseudonymize`.

Check profiles before you ship a policy: for example, reject a recoverable
operator on a label your policy says must be destroyed.

## Your own operators

An operator implements `Operator<M>`; one that can be undone also implements
`ReversibleOperator<M>`. Both are in `elide::redaction`.
