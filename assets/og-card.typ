#set page(width: 1200pt, height: 630pt, margin: 0pt)
#set text(font: ("JetBrains Mono", "JetBrainsMono NF", "DejaVu Sans Mono"), fill: rgb("#e0e0e0"))

#let bg = rgb("#0a0a0a")
#let surface = rgb("#141414")
#let green = rgb("#00ff88")
#let red = rgb("#ff4444")
#let amber = rgb("#ffaa00")
#let muted = rgb("#666666")
#let border = rgb("#333333")
#let white = rgb("#ffffff")

#let symbol = sys.inputs.at("symbol", default: "ASX")
#let title = sys.inputs.at("title", default: "Announcement")
#let ann-type = sys.inputs.at("type", default: "General")
#let summary = sys.inputs.at("summary", default: "")
#let is-sensitive = sys.inputs.at("sensitive", default: "false")

#block(width: 100%, height: 100%, fill: bg)[
  #block(width: 100%, height: 4pt, fill: green)

  #grid(
    columns: (280pt, 1fr),
    rows: (100%),

    // Left column — symbol + branding
    block(width: 100%, height: 100%, fill: surface, inset: (x: 40pt, y: 40pt))[
      #text(size: 64pt, weight: "bold", fill: green)[
        \$#symbol
      ]

      #v(16pt)

      #if is-sensitive == "true" [
        #box(
          fill: amber.transparentize(80%),
          inset: (x: 12pt, y: 6pt),
        )[
          #text(size: 14pt, weight: "bold", fill: amber)[⚡ PRICE SENSITIVE]
        ]
        #v(12pt)
      ]

      #box(
        fill: bg,
        inset: (x: 12pt, y: 6pt),
        stroke: 1pt + border,
      )[
        #text(size: 12pt, fill: muted)[#upper(ann-type)]
      ]

      #v(1fr)

      #text(size: 22pt, weight: "bold", fill: green)[STONKWATCH]
      #v(4pt)
      #text(size: 12pt, fill: muted)[ASX Market Intelligence]
      #v(8pt)
      #text(size: 11pt, fill: muted)[stonkwatch.app]
    ],

    // Right column — title + summary
    block(width: 100%, height: 100%, fill: bg, inset: (x: 48pt, y: 48pt))[
      #text(size: 30pt, weight: "bold", fill: white)[
        #title
      ]

      #v(24pt)

      #if summary != "" [
        #block(
          width: 100%,
          fill: surface,
          inset: 20pt,
          stroke: (left: 3pt + green),
        )[
          #text(size: 16pt, fill: rgb("#cccccc"), weight: "regular")[
            #summary
          ]
        ]
      ] else [
        #block(
          width: 100%,
          fill: surface,
          inset: 20pt,
          stroke: (left: 3pt + green),
        )[
          #text(size: 16pt, fill: rgb("#cccccc"))[
            New #ann-type announcement from \$#symbol on the ASX. Read the full AI analysis on Stonkwatch.
          ]
        ]
      ]

      #v(1fr)

      #text(size: 13pt, fill: muted)[
        AI-powered analysis available → stonkwatch.app
      ]
    ],
  )
]
