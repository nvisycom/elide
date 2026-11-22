"""An invoice, drawn with ReportLab: standard fonts it embeds no program for,
a TrueType font it embeds, and text state ReportLab sets itself (character
and word spacing, horizontal scaling, rise, a rotated line).

Usage: invoice.py FONTS_DIR OUT.pdf
"""

import sys

from reportlab.lib.pagesizes import A4
from reportlab.pdfbase import pdfmetrics
from reportlab.pdfbase.ttfonts import TTFont
from reportlab.pdfgen import canvas

fonts, out = sys.argv[1], sys.argv[2]
pdfmetrics.registerFont(TTFont("ElideTestSans", f"{fonts}/ElideTestSans.ttf"))
pdfmetrics.registerFont(TTFont("ElideTestMono", f"{fonts}/ElideTestMono.ttf"))

c = canvas.Canvas(out, pagesize=A4, invariant=1)
width, height = A4

c.setFont("Helvetica-Bold", 18)
c.drawString(56, height - 72, "INVOICE  No. 2026-0381")
c.setFont("Times-Roman", 11)
c.drawString(56, height - 96, "Billed to: Priya Natarajan, 9 Rue des Lilas, 75011 Paris")
c.drawString(56, height - 112, "VAT FR40 303 265 045 · priya.natarajan@example.fr · +33 1 70 18 99 00")

text = c.beginText(56, height - 150)
text.setFont("ElideTestSans", 10)
text.setCharSpace(0.6)
text.textLine("Line items are listed below. Payment is due within 30 days.")
text.setCharSpace(0)
text.setWordSpace(4)
text.textLine("Please transfer to IBAN FR76 3000 6000 0112 3456 7890 189 quoting 2026-0381.")
text.setWordSpace(0)
text.setHorizScale(80)
text.textLine("Condensed: card on file ends 0004, expires 09/29.")
text.setHorizScale(100)
text.setRise(3)
text.textOut("Raised note")
text.setRise(0)
text.textLine(" and back on the baseline.")
c.drawText(text)

y = height - 240
c.setFont("ElideTestMono", 9)
for description, amount in [
    ("Consulting, 12 h @ 95.00", "1,140.00"),
    ("Travel: Paris–Lyon return", "  212.40"),
    ("Hosting, account pn-7731", "   48.00"),
]:
    c.drawString(56, y, f"{description:<34}{amount:>12}")
    y -= 14

c.setFont("Courier", 9)
c.drawString(56, y - 10, "Total due (EUR)                     1,400.40")

c.saveState()
c.translate(30, 120)
c.rotate(90)
c.setFont("Helvetica", 7)
c.drawString(0, 0, "Account holder: Priya Natarajan · customer ID PN-7731")
c.restoreState()

c.showPage()
c.setFont("Helvetica", 11)
c.drawString(56, height - 72, "Page 2 — terms. Contact billing@example.com for questions.")
c.save()
