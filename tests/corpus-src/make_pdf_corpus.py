"""Generates valid K.26 corpus PDFs and deterministic K.23 malformed fuzz regression seeds.

Run from the repository root: `python tests/corpus-src/make_pdf_corpus.py`.
Every file is hand-crafted with zero third-party content.
"""

import gzip
import zipfile
import zlib
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
CORPUS = ROOT / "corpus"
FUZZ_CORPUS = ROOT / "fuzz" / "corpus"


def build_pdf(objects: list[bytes], catalog_id: int = 1) -> bytes:
    """Builds a standards-compliant PDF 1.7 with exact byte-offset xref table."""
    chunks = [b"%PDF-1.7\n%\xe2\xe3\xcf\xd3\n"]
    offsets = [0]
    offset = len(chunks[0])
    for idx, body in enumerate(objects, start=1):
        obj = f"{idx} 0 obj\n".encode("ascii") + body + b"\nendobj\n"
        offsets.append(offset)
        chunks.append(obj)
        offset += len(obj)

    xref_offset = offset
    xref = [f"xref\n0 {len(objects) + 1}\n0000000000 65535 f \n".encode("ascii")]
    for off in offsets[1:]:
        xref.append(f"{off:010d} 00000 n \n".encode("ascii"))
    trailer = (
        f"trailer\n<< /Size {len(objects) + 1} /Root {catalog_id} 0 R >>\n"
        f"startxref\n{xref_offset}\n%%EOF\n"
    ).encode("ascii")
    return b"".join(chunks + xref + [trailer])


def make_rtl_pdf() -> bytes:
    stream = b"BT /F1 18 Tf 72 740 Td (RTL Right-to-Left ViewerPreferences Test) Tj ET"
    objects = [
        b"<< /Type /Catalog /Pages 2 0 R /ViewerPreferences << /Direction /R2L >> >>",
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 595 842] /Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R >>",
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>",
        f"<< /Length {len(stream)} >>\nstream\n".encode("ascii") + stream + b"\nendstream",
    ]
    return build_pdf(objects)


def make_acroform_pdf() -> bytes:
    stream = (
        b"BT /F1 16 Tf 72 740 Td (AcroForm Interop Sample) Tj ET\n"
        b"0.2 w 72 680 220 24 re S"
    )
    objects = [
        b"<< /Type /Catalog /Pages 2 0 R /AcroForm << /Fields [6 0 R] /NeedAppearances true /DA (/Helv 12 Tf 0 g) /DR << /Font << /Helv 4 0 R >> >> >> >>",
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R /Annots [6 0 R] >>",
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>",
        f"<< /Length {len(stream)} >>\nstream\n".encode("ascii") + stream + b"\nendstream",
        b"<< /Type /Annot /Subtype /Widget /FT /Tx /T (FullName) /V (Ada Lovelace) /DA (/Helv 12 Tf 0 g) /Rect [72 680 292 704] /P 3 0 R /F 4 >>",
    ]
    return build_pdf(objects)


def make_ten_page_paper() -> bytes:
    # 1: Catalog, 2: Pages, 3: Font, 4: Outlines, 5..14: Pages, 15..24: Contents, 25: Outline item
    objects: list[bytes] = []
    kids = " ".join(f"{5 + i} 0 R" for i in range(10))
    objects.append(
        b"<< /Type /Catalog /Pages 2 0 R /Outlines 4 0 R /PageLabels << /Nums [0 << /S /D >>] >> >>"
    )
    objects.append(f"<< /Type /Pages /Kids [{kids}] /Count 10 >>".encode("ascii"))
    objects.append(b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>")
    objects.append(b"<< /Type /Outlines /First 25 0 R /Last 25 0 R /Count 1 >>")
    for i in range(10):
        content_id = 15 + i
        objects.append(
            f"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 3 0 R >> >> /Contents {content_id} 0 R >>".encode(
                "ascii"
            )
        )
    for i in range(10):
        stream = (
            f"BT /F1 14 Tf 72 720 Td (EcclesiaPDF Benchmark Paper - Page {i + 1} of 10) Tj ET\n"
            f"BT /F1 11 Tf 72 690 Td (Paragraph sample for budget, text, and golden layout sweeps.) Tj ET"
        ).encode("ascii")
        objects.append(f"<< /Length {len(stream)} >>\nstream\n".encode("ascii") + stream + b"\nendstream")
    objects.append(b"<< /Title (Introduction) /Parent 4 0 R /Dest [5 0 R /Fit] >>")
    return build_pdf(objects)


def write_fuzz_seeds() -> None:
    pdf_dir = FUZZ_CORPUS / "malformed_pdf"
    xfdf_dir = FUZZ_CORPUS / "malformed_xfdf"
    expdf_dir = FUZZ_CORPUS / "malformed_expdf"
    synctex_dir = FUZZ_CORPUS / "malformed_synctex"
    for d in (pdf_dir, xfdf_dir, expdf_dir, synctex_dir):
        d.mkdir(parents=True, exist_ok=True)

    (pdf_dir / "truncated.pdf").write_bytes(b"%PDF-1.7\n1 0 obj\n<< /Type ")
    (pdf_dir / "deep_nest.pdf").write_bytes(
        b"%PDF-1.7\n1 0 obj " + b"<< /A " * 80 + b"1" + b" >>" * 80 + b" endobj\n%%EOF"
    )
    (pdf_dir / "huge_xref.pdf").write_bytes(b"%PDF-1.7\ntrailer << /Size 99999999 >>\n%%EOF")
    compressed_bomb = zlib.compress(b"\x00" * (256 * 1024), level=9)
    (pdf_dir / "zlib_bomb.pdf").write_bytes(
        b"%PDF-1.7\n1 0 obj << /Length 10 >>\nstream\n" + compressed_bomb + b"\nendstream\nendobj\n%%EOF"
    )

    (xfdf_dir / "unclosed.xfdf").write_bytes(b"<?xml version='1.0'?><xfdf><annots><text")
    (xfdf_dir / "entity_bomb.xfdf").write_bytes(
        b'<?xml version="1.0"?><!DOCTYPE x ['
        + b"".join(f'<!ENTITY e{i} "x">'.encode("ascii") for i in range(20))
        + b"]><xfdf/>"
    )

    traversal_zip = expdf_dir / "traversal.expdf"
    with zipfile.ZipFile(traversal_zip, "w") as zf:
        zf.writestr("../../outside.txt", b"escape")
    (expdf_dir / "truncated.expdf").write_bytes(b"PK\x03\x04\x14\x00\x00")

    (synctex_dir / "bad_header.synctex").write_bytes(b"NotSyncTeX\n1,2,3\n")
    (synctex_dir / "truncated.synctex.gz").write_bytes(b"\x1f\x8b\x08\x00\x00\x00\x00\x00")
    (synctex_dir / "valid_sample.synctex.gz").write_bytes(
        gzip.compress(b"SyncTeX Version:1\nInput:1:main.tex\nOutput:main.pdf\n")
    )


def main() -> None:
    (CORPUS / "rtl-arabic-biditest.pdf").write_bytes(make_rtl_pdf())
    (CORPUS / "acroform-fields.pdf").write_bytes(make_acroform_pdf())
    (CORPUS / "ten-page-paper.pdf").write_bytes(make_ten_page_paper())
    write_fuzz_seeds()


if __name__ == "__main__":
    main()
