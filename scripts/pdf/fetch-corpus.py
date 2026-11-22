#!/usr/bin/env python3
"""Fetch the real documents elide-pdf's placement test reads.

`crates/elide-pdf/testdata/corpus/fetched.tsv` lists each document: its name,
where it is from, its URL (with the member of a govdocs1 zip, read by range
requests rather than downloaded whole), its pinned SHA-256 and its licence. A
document already fetched with the pinned hash is kept; one that doesn't match
its hash is an error. The documents are third-party and stay out of the
repository; the test reads them from the directory `ELIDE_PDF_CORPUS` names.

Usage:

    scripts/pdf/fetch-corpus.py [DIR]

where `DIR` defaults to `target/pdf-corpus`.
"""

import hashlib
import io
import sys
import urllib.request
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
MANIFEST = ROOT / "crates/elide-pdf/testdata/corpus/fetched.tsv"
# Bytes read past each request, so a member's local header and data share one.
READ_AHEAD = 1 << 16


class Remote(io.RawIOBase):
    """A file served over HTTP, read by range requests."""

    def __init__(self, url):
        self.url = url
        request = urllib.request.Request(url, method="HEAD")
        with urllib.request.urlopen(request) as response:
            self.size = int(response.headers["Content-Length"])
        self.position = 0
        self.start = 0
        self.buffer = b""

    def seekable(self):
        return True

    def readable(self):
        return True

    def tell(self):
        return self.position

    def seek(self, offset, whence=io.SEEK_SET):
        base = {io.SEEK_SET: 0, io.SEEK_CUR: self.position, io.SEEK_END: self.size}
        self.position = base[whence] + offset
        return self.position

    def read(self, size=-1):
        end = self.size if size < 0 else min(self.size, self.position + size)
        if end <= self.position:
            return b""
        if not (self.start <= self.position and end <= self.start + len(self.buffer)):
            last = min(self.size, end + READ_AHEAD) - 1
            request = urllib.request.Request(
                self.url, headers={"Range": f"bytes={self.position}-{last}"}
            )
            with urllib.request.urlopen(request) as response:
                self.buffer = response.read()
            self.start = self.position
        data = self.buffer[self.position - self.start : end - self.start]
        self.position += len(data)
        return data


def main():
    out = Path(sys.argv[1]) if len(sys.argv) > 1 else ROOT / "target/pdf-corpus"
    out.mkdir(parents=True, exist_ok=True)
    entries = [
        line.rstrip("\n").split("\t")
        for line in MANIFEST.read_text(encoding="utf-8").splitlines()
        if line.strip() and not line.startswith("#")
    ]
    zips = {}
    fetched = kept = 0
    for name, _source, url, member, sha256, _licence in entries:
        path = out / name
        if path.exists() and hashlib.sha256(path.read_bytes()).hexdigest() == sha256:
            kept += 1
            continue
        if member != "-":
            if url not in zips:
                zips[url] = zipfile.ZipFile(Remote(url))
            data = zips[url].read(member)
        else:
            with urllib.request.urlopen(url) as response:
                data = response.read()
        digest = hashlib.sha256(data).hexdigest()
        if digest != sha256:
            sys.exit(f"{name}: SHA-256 {digest}, pinned {sha256}")
        path.write_bytes(data)
        fetched += 1
    print(f"{fetched} fetched, {kept} already in {out}")


if __name__ == "__main__":
    main()
