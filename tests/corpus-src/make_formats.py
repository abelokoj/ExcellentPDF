"""Writes the `formats.*` files of tests/corpus (plan R.28): one small document per format MuPDF opens besides PDF.

Run from the repository root: `python tests/corpus-src/make_formats.py`. The text is EcclesiaPDF's own; every file
is the same short book, so the tests can find the same words in each.
"""

import struct
import zipfile
import zlib
from pathlib import Path

OUT = Path(__file__).resolve().parent.parent / "corpus"
TITLE = "Formats"

# Each sentence carries a marker such as "2.07" (chapter 2, sentence 7), so a test can tell where a page starts.
WORDS = (
    "the reader turns a page and the lamp hums while rain taps the window and the story goes on "
    "through rooms of quiet paper where each line waits its turn to be read"
).split()


def sentence(chapter: int, number: int) -> str:
    start = (chapter * 7 + number * 3) % len(WORDS)
    body = " ".join(WORDS[(start + i) % len(WORDS)] for i in range(14))
    return f"Mark {chapter}.{number:02d} says {body}."


def chapters() -> list[tuple[str, list[str]]]:
    book = []
    for chapter in range(1, 4):
        paragraphs = []
        for paragraph in range(8):
            first = paragraph * 6
            paragraphs.append(" ".join(sentence(chapter, first + i) for i in range(6)))
        book.append((f"Chapter {chapter}", paragraphs))
    return book


def xhtml(heading: str, paragraphs: list[str], lead: str = "") -> str:
    body = lead + "\n".join(f"<p>{p}</p>" for p in paragraphs)
    return (
        '<?xml version="1.0" encoding="utf-8"?>\n'
        '<html xmlns="http://www.w3.org/1999/xhtml"><head><title>'
        f"{heading}</title></head><body>\n<h1>{heading}</h1>\n{body}\n</body></html>\n"
    )


def stored_zip(path: Path, files: list[tuple[str, bytes]], first_stored: bool) -> None:
    with zipfile.ZipFile(path, "w") as archive:
        for index, (name, data) in enumerate(files):
            stored = first_stored and index == 0
            method = zipfile.ZIP_STORED if stored else zipfile.ZIP_DEFLATED
            info = zipfile.ZipInfo(name, date_time=(2026, 11, 14, 0, 0, 0))
            info.compress_type = method
            archive.writestr(info, data)


def epub(book) -> None:
    files = [("mimetype", b"application/epub+zip")]
    files.append((
        "META-INF/container.xml",
        b'<?xml version="1.0"?>\n<container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container">'
        b'<rootfiles><rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/>'
        b"</rootfiles></container>\n",
    ))
    items = "".join(
        f'<item id="c{i}" href="c{i}.xhtml" media-type="application/xhtml+xml"/>' for i in range(len(book))
    )
    spine = "".join(f'<itemref idref="c{i}"/>' for i in range(len(book)))
    opf = (
        '<?xml version="1.0" encoding="utf-8"?>\n'
        '<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="id">'
        '<metadata xmlns:dc="http://purl.org/dc/elements/1.1/">'
        f'<dc:identifier id="id">ecclesiapdf-formats</dc:identifier><dc:title>{TITLE}</dc:title>'
        "<dc:language>en</dc:language></metadata>"
        f"<manifest>{items}</manifest><spine>{spine}</spine></package>\n"
    )
    files.append(("OEBPS/content.opf", opf.encode()))
    # R.7: the first chapter links to the last one and to the web.
    links = '<p>See <a href="c2.xhtml">the last chapter</a> or <a href="https://example.com/">the web</a>.</p>\n'
    for i, (heading, paragraphs) in enumerate(book):
        files.append((f"OEBPS/c{i}.xhtml", xhtml(heading, paragraphs, links if i == 0 else "").encode()))
    stored_zip(OUT / "formats.epub", files, first_stored=True)


def fb2(book) -> None:
    sections = "".join(
        f"<section><title><p>{heading}</p></title>" + "".join(f"<p>{p}</p>" for p in paragraphs) + "</section>"
        for heading, paragraphs in book
    )
    text = (
        '<?xml version="1.0" encoding="utf-8"?>\n'
        '<FictionBook xmlns="http://www.gribuser.ru/xml/fictionbook/2.0"><description><title-info>'
        f"<book-title>{TITLE}</book-title><lang>en</lang></title-info></description>"
        f"<body>{sections}</body></FictionBook>\n"
    )
    (OUT / "formats.fb2").write_bytes(text.encode())


def html_body(book) -> str:
    return "\n".join(
        f"<h1>{heading}</h1>\n" + "\n".join(f"<p>{p}</p>" for p in paragraphs) for heading, paragraphs in book
    )


def html(book) -> None:
    text = f"<!DOCTYPE html>\n<html><head><meta charset=\"utf-8\"><title>{TITLE}</title></head><body>\n"
    (OUT / "formats.html").write_bytes((text + html_body(book) + "\n</body></html>\n").encode())


def markdown(book) -> None:
    text = "\n\n".join(f"# {heading}\n\n" + "\n\n".join(paragraphs) for heading, paragraphs in book)
    (OUT / "formats.md").write_bytes((text + "\n").encode())


def mobi(book) -> None:
    """A PalmDB holding uncompressed UTF-8 HTML, as MuPDF's `mobi.c` reads it."""
    text = f"<html><body>{html_body(book)}</body></html>".encode()
    records = [text[i : i + 4096] for i in range(0, len(text), 4096)]
    # Record 0: the PalmDOC header (no compression, the text's length, its record count, 4096-byte records), then a
    # MOBI header naming UTF-8.
    palmdoc = struct.pack(">HHIHHHH", 1, 0, len(text), len(records), 4096, 0, 0)
    mobi_header = b"MOBI" + struct.pack(">III", 24, 2, 65001) + b"\0" * 8
    all_records = [palmdoc + mobi_header] + records
    count = len(all_records)
    header = TITLE.encode().ljust(32, b"\0") + b"\0" * 28 + b"BOOKMOBI" + b"\0" * 8 + struct.pack(">H", count)
    offset = len(header) + count * 8 + 2
    listing = b""
    for index, record in enumerate(all_records):
        listing += struct.pack(">II", offset, index * 2)
        offset += len(record)
    (OUT / "formats.mobi").write_bytes(header + listing + b"\0\0" + b"".join(all_records))


def png(width: int, height: int, shade: int) -> bytes:
    """A white picture with a dark band across its middle."""
    rows = b""
    for y in range(height):
        dark = height // 3 <= y < 2 * height // 3
        value = shade if dark else 255
        rows += b"\0" + bytes([value, value, value]) * width

    def chunk(kind: bytes, data: bytes) -> bytes:
        return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data))

    ihdr = struct.pack(">IIBBBBB", width, height, 8, 2, 0, 0, 0)
    return b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", ihdr) + chunk(b"IDAT", zlib.compress(rows)) + chunk(b"IEND", b"")


def svg() -> None:
    text = (
        '<?xml version="1.0" encoding="utf-8"?>\n'
        '<svg xmlns="http://www.w3.org/2000/svg" width="420" height="300" viewBox="0 0 420 300">'
        '<rect x="20" y="20" width="380" height="120" fill="#203040"/>'
        f'<text x="20" y="200" font-family="Helvetica" font-size="24" fill="#000">{sentence(1, 0)[:30]}</text>'
        "</svg>\n"
    )
    (OUT / "formats.svg").write_bytes(text.encode())


def xps() -> None:
    ns = "http://schemas.microsoft.com/xps/2005/06"
    files = [
        (
            "[Content_Types].xml",
            b'<?xml version="1.0" encoding="utf-8"?>'
            b'<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">'
            b'<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>'
            b'<Default Extension="fdseq" ContentType="application/vnd.ms-package.xps-fixeddocumentsequence+xml"/>'
            b'<Default Extension="fdoc" ContentType="application/vnd.ms-package.xps-fixeddocument+xml"/>'
            b'<Default Extension="fpage" ContentType="application/vnd.ms-package.xps-fixedpage+xml"/></Types>',
        ),
        (
            "_rels/.rels",
            b'<?xml version="1.0" encoding="utf-8"?>'
            b'<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">'
            b'<Relationship Id="R0" Target="/FixedDocSeq.fdseq" '
            b'Type="http://schemas.microsoft.com/xps/2005/06/fixedrepresentation"/></Relationships>',
        ),
        (
            "FixedDocSeq.fdseq",
            f'<FixedDocumentSequence xmlns="{ns}"><DocumentReference Source="/Documents/1/FixedDoc.fdoc"/>'
            "</FixedDocumentSequence>".encode(),
        ),
        (
            "Documents/1/FixedDoc.fdoc",
            f'<FixedDocument xmlns="{ns}"><PageContent Source="Pages/1.fpage"/></FixedDocument>'.encode(),
        ),
        (
            "Documents/1/Pages/1.fpage",
            f'<FixedPage Width="816" Height="1056" xmlns="{ns}" xml:lang="en">'
            # R.7: the shape is a link to the web.
            '<Path Data="M 96,96 L 720,96 720,320 96,320 Z" Fill="#FF203040" '
            'FixedPage.NavigateUri="https://example.com/"/></FixedPage>'.encode(),
        ),
    ]
    stored_zip(OUT / "formats.xps", files, first_stored=False)


def cbz() -> None:
    files = [("01.png", png(120, 160, 40)), ("02.png", png(120, 160, 90))]
    stored_zip(OUT / "formats.cbz", files, first_stored=False)


def main() -> None:
    book = chapters()
    epub(book)
    fb2(book)
    html(book)
    markdown(book)
    mobi(book)
    svg()
    xps()
    cbz()
    (OUT / "formats.png").write_bytes(png(160, 120, 30))


if __name__ == "__main__":
    main()
