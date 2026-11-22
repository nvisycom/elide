# Operators

An operator produces the replacement for one entity. They live in
`elide::redaction::operators`, also reachable as
`elide::prelude::operators`.

## Text

For `dana.reed@example.com`:

| Operator | Result |
|---|---|
| `Keep` | Left as it is: nothing is written; the selection is audited |
| `Remove` | Removed: nothing marks where it was |
| `Cover::default()` | `█████████████████████`, one `█` per character |
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
| `Image` | `Blur::new(sigma)`, `Pixelate::new(block_size)` |
| `Audio` | `Beep::new(hz)` |
| `Tabular` | `DropRow`, `DropColumn`, and the text operators on a cell |

`Keep` works in every modality, `Cover` in every built-in one, and `Remove`
in every one that can make a value vanish. `Remove` leaves nothing that marks
where the value was: text closes up, and audio is cut. An image can't lose its
pixels, so `Remove` doesn't apply to it; use `Cover`, which hides a value in
place and keeps its extent: text is covered, audio silenced, an image region
covered in black, or in the colour given to `Cover::new(color)`. A custom
modality implements `Operator` for `Remove` and `Cover` itself, for whichever
it can give.

## Leak profiles

Every operator declares a `LeakProfile`, how much of the original value its
output still exposes: its content, or its extent. Where the value sat (which
sentence, cell, or second of audio) is not counted, since every treatment
keeps its surroundings.

- **Irrecoverable**: nothing of the value survives. `Remove`, `DropRow`,
  `DropColumn`.
- **Partial**: part of the value survives by design: some of its content (the
  digits a mask keeps, a date's year, a bucket, a blur) or its extent (a
  cover's length, a box's size). `Cover`, `Mask`, `Replace`, `Truncate`,
  `Clamp`, `GeneralizeDate`, and the image and audio treatments.
- **Recoverable**: the output stays linked to the original. `AesEncrypt`
  decrypts with its key; `Pseudonymize`'s vault keeps the mapping from each
  original (or coreference cluster) to its surrogate; a hash can be
  brute-forced over a small input space; `Keep` leaves the original in place,
  and a `Replace` template with `{value}` writes it back.

Check profiles before you ship a policy: for example, reject a recoverable
operator on a label your policy says must be destroyed.

## Your own operators

An operator implements `Operator<M>`: its `anonymize` returns the replacement
to write, or `None` to leave the entity as it is. One that can be undone also
implements `ReversibleOperator<M>`. Both are in `elide::redaction`.
