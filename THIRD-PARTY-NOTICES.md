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

`elide-pdf` reads the widths of the standard 14 fonts from Adobe's Core14 AFM files, shipped unmodified in `crates/elide-pdf/assets/afm` with Adobe's `MustRead.html`. Any binary built with `elide-pdf` contains them.

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

`elide-pdf` reads the text a glyph name stands for from the Adobe Glyph List and its ZapfDingbats list, shipped unmodified in `crates/elide-pdf/assets/agl`. Any binary built with `elide-pdf` contains them.

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
