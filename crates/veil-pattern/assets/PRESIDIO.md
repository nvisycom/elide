# Presidio attribution

Several shipped pattern TOMLs under `patterns/` carry regular
expressions ported or adapted from [Microsoft Presidio][presidio]
(`microsoft/presidio`, MIT-licensed) — specifically the
`presidio-analyzer/presidio_analyzer/predefined_recognizers/`
classes. Validators (Luhn, IBAN mod-97, ABA, DEA, NPI, NHS,
NINO, etc.) were re-implemented in Rust from the same upstream
algorithms.

The Presidio MIT license text is reproduced below to satisfy its
"include this permission notice in all copies or substantial
portions" clause.

[presidio]: https://github.com/microsoft/presidio

---

```
MIT License

Copyright (c) Microsoft Corporation.

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```
