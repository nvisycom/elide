# Third-party notices

Elide is licensed under Apache-2.0 (see [LICENSE](LICENSE)). Some of its dependencies carry data or code under licenses that require their notice to be reproduced when Elide is distributed in binary form. Those notices follow.

## Adobe CMap resources

`elide-pdf` reads the encodings of CJK fonts with `hayro-cmap`, which bundles Adobe's predefined CMap resources (via [cmap-resources](https://github.com/LaurenzV/cmap-resources/tree/hayro)). Any binary built with `elide-pdf` contains them.

```text
Copyright 1990-2023 Adobe. All rights reserved.

Redistribution and use in source and binary forms, with or without
modification, are permitted provided that the following conditions are
met:

Redistributions of source code must retain the above copyright notice,
this list of conditions and the following disclaimer.

Redistributions in binary form must reproduce the above copyright
notice, this list of conditions and the following disclaimer in the
documentation and/or other materials provided with the distribution.

Neither the name of Adobe nor the names of its contributors may be
used to endorse or promote products derived from this software without
specific prior written permission.

THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS
"AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT
LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR
A PARTICULAR PURPOSE ARE DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT
HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT
LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR SERVICES; LOSS OF USE,
DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY
THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT
(INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE
OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.
```

## Adobe Core14 AFM files

`elide-pdf` takes the metrics of the standard 14 fonts from Adobe's Core14 AFM files: each glyph's width and built-in code, and each face's vertical extent, bounding box, italic angle and stem. They are generated, modified as the file notes, into `crates/elide-pdf/src/pdf/reader/font/standard/metrics.rs` by `scripts/pdf/gen-metrics.mjs`, with Adobe's `MustRead.html` unmodified beside it. Any binary built with `elide-pdf` contains them.

```text
Copyright (c) 1985, 1987, 1988, 1989, 1990, 1991, 1992, 1993, 1997 Adobe
Systems Incorporated. All Rights Reserved.

This file and the 14 PostScript(R) AFM files it accompanies may be used,
copied, and distributed for any purpose and without charge, with or without
modification, provided that all copyright notices are retained; that the AFM
files are not distributed without this file; that all modifications to this
file or any of the AFM files are prominently noted in the modified file(s);
and that this paragraph is not modified. Adobe Systems has no responsibility
or obligation to support the use of the AFM files.
```

## Adobe Glyph List

`elide-pdf` reads the text a glyph name stands for from the Adobe Glyph List and its ZapfDingbats list, shipped unmodified in `crates/elide-pdf/assets/agl`. It reads embedded Type 1 programs with `read-fonts`, which bundles the same two lists. Any binary built with `elide-pdf` contains them.

```text
Copyright 2002-2019 Adobe (http://www.adobe.com/).

Redistribution and use in source and binary forms, with or without
modification, are permitted provided that the following conditions are
met:

Redistributions of source code must retain the above copyright notice,
this list of conditions and the following disclaimer.

Redistributions in binary form must reproduce the above copyright
notice, this list of conditions and the following disclaimer in the
documentation and/or other materials provided with the distribution.

Neither the name of Adobe nor the names of its contributors may be
used to endorse or promote products derived from this software without
specific prior written permission.

THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS
"AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT
LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR
A PARTICULAR PURPOSE ARE DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT
HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT
LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR SERVICES; LOSS OF USE,
DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY
THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT
(INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE
OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.
```

## LAME

`elide-audio` encodes MP3 with LAME, through `mp3lame-sys`, which compiles it from source and links it statically. It is built only with `elide-audio`'s opt-in `mp3` feature (`codec-mp3` or `codec-audio` on `elide`); a binary built without it contains no LAME code.

LAME is licensed under the GNU Library General Public License, version 2 or (at your option) any later version; `mp3lame-sys` distributes it under version 3. A binary that contains it must be accompanied by that license's text and must let its recipient relink it against a modified LAME, for example by providing the application's object files. LAME's source is at <https://lame.sourceforge.io/>, and the version built here is the one `mp3lame-sys` bundles, at <https://github.com/DoumanAsh/mp3lame-sys>.

```text
LAME MP3 encoding engine

Copyright (c) 1999-2000 Mark Taylor
Copyright (c) 2000-2005 Takehiro Tominaga
Copyright (c) 2000-2019 Robert Hegemann
Copyright (c) 2000-2005 Gabriel Bouvigne
Copyright (c) 2000-2004 Alexander Leidinger

This library is free software; you can redistribute it and/or
modify it under the terms of the GNU Library General Public
License as published by the Free Software Foundation; either
version 2 of the License, or (at your option) any later version.

This library is distributed in the hope that it will be useful,
but WITHOUT ANY WARRANTY; without even the implied warranty of
MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the GNU
Library General Public License for more details.
```

## Microsoft Presidio

`elide-pattern` ships regular expressions ported or adapted from the predefined recognizers of [Microsoft Presidio](https://github.com/microsoft/presidio), and validators reimplemented from the same upstream algorithms (see `crates/elide-pattern/assets/PRESIDIO.md`). The patterns are compiled into the crate, so any binary built with `elide-pattern` contains them.

```text
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

## Mozilla CA certificates

`elide-llm`'s `rig` backend connects to model providers over TLS with `rustls`, through `rig-reqwest`, which bundles Mozilla's root certificate store (via [webpki-roots](https://github.com/rustls/webpki-roots)'s `webpki-root-certs`). Any binary built with that backend contains it: the `rig` feature, or a provider feature that enables it (`openai-gpt`, `anthropic-claude`, `google-gemini`; `llm-rig` on `elide`). A binary built with only the `mocks` backend does not. The store is shared under the Community Data License Agreement – Permissive 2.0, whose text must accompany it:

```text
# Community Data License Agreement - Permissive - Version 2.0

This is the Community Data License Agreement - Permissive, Version
2.0 (the "agreement"). Data Provider(s) and Data Recipient(s) agree
as follows:

## 1. Provision of the Data

1.1. A Data Recipient may use, modify, and share the Data made
available by Data Provider(s) under this agreement if that Data
Recipient follows the terms of this agreement.

1.2. This agreement does not impose any restriction on a Data
Recipient's use, modification, or sharing of any portions of the
Data that are in the public domain or that may be used, modified,
or shared under any other legal exception or limitation.

## 2. Conditions for Sharing Data

2.1. A Data Recipient may share Data, with or without modifications, so
long as the Data Recipient makes available the text of this agreement
with the shared Data.

## 3. No Restrictions on Results

3.1. This agreement does not impose any restriction or obligations
with respect to the use, modification, or sharing of Results.

## 4. No Warranty; Limitation of Liability

4.1. All Data Recipients receive the Data subject to the following
terms:

THE DATA IS PROVIDED ON AN "AS IS" BASIS, WITHOUT REPRESENTATIONS,
WARRANTIES OR CONDITIONS OF ANY KIND, EITHER EXPRESS OR IMPLIED
INCLUDING, WITHOUT LIMITATION, ANY WARRANTIES OR CONDITIONS OF TITLE,
NON-INFRINGEMENT, MERCHANTABILITY OR FITNESS FOR A PARTICULAR PURPOSE.

NO DATA PROVIDER SHALL HAVE ANY LIABILITY FOR ANY DIRECT, INDIRECT,
INCIDENTAL, SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING
WITHOUT LIMITATION LOST PROFITS), HOWEVER CAUSED AND ON ANY THEORY OF
LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING
NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE DATA OR RESULTS,
EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGES.

## 5. Definitions

5.1. "Data" means the material received by a Data Recipient under
this agreement.

5.2. "Data Provider" means any person who is the source of Data
provided under this agreement and in reliance on a Data Recipient's
agreement to its terms.

5.3. "Data Recipient" means any person who receives Data directly
or indirectly from a Data Provider and agrees to the terms of this
agreement.

5.4. "Results" means any outcome obtained by computational analysis
of Data, including for example machine learning models and models'
insights.
```

## Symphonia

`elide-audio` decodes audio with [Symphonia](https://github.com/pdeljanov/Symphonia), licensed under the Mozilla Public License 2.0. Any binary built with `elide-audio`'s `wav` or `mp3` feature (`codec-wav`, `codec-mp3` or `codec-audio` on `elide`) contains it; one built with neither does not. Its source, including the version built here, is available at <https://github.com/pdeljanov/Symphonia>, and the license at <https://mozilla.org/MPL/2.0/>.

```text
Copyright (c) 2019-2026 The Project Symphonia Developers.

This Source Code Form is subject to the terms of the Mozilla Public
License, v. 2.0. If a copy of the MPL was not distributed with this
file, You can obtain one at https://mozilla.org/MPL/2.0/.
```
