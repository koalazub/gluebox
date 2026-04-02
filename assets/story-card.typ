#set page(width: 1080pt, height: 1920pt, margin: 0pt)
#set text(font: "JetBrains Mono", fill: rgb("#e0e0e0"))

#let bg = rgb("#0a0a0a")
#let surface = rgb("#141414")
#let green = rgb("#00ff88")
#let amber = rgb("#ffaa00")
#let muted = rgb("#666666")
#let border = rgb("#333333")
#let white = rgb("#ffffff")

#let symbol = sys.inputs.at("symbol", default: "ASX")
#let title = sys.inputs.at("title", default: "Announcement")
#let ann-type = sys.inputs.at("type", default: "General")
#let summary = sys.inputs.at("summary", default: "")
#let is-sensitive = sys.inputs.at("sensitive", default: "false")
#let link = sys.inputs.at("link", default: "stonkwatch.app")

#block(width: 100%, height: 100%, fill: bg)[
  #block(width: 100%, height: 6pt, fill: green)

  #pad(x: 64pt, top: 200pt, bottom: 120pt)[

    #text(size: 80pt, weight: "bold", fill: green)[
      \$#symbol
    ]

    #v(24pt)

    #if is-sensitive == "true" [
      #box(
        fill: amber.transparentize(80%),
        inset: (x: 16pt, y: 10pt),
      )[
        #text(size: 28pt, weight: "bold", fill: amber)[⚡ PRICE SENSITIVE]
      ]
      #v(24pt)
    ]

    #box(
      fill: surface,
      inset: (x: 14pt, y: 8pt),
      stroke: 1pt + border,
    )[
      #text(size: 18pt, fill: muted)[#upper(ann-type)]
    ]

    #v(48pt)

    #text(size: 44pt, weight: "bold", fill: white)[
      #title
    ]

    #v(40pt)

    #if summary != "" [
      #block(
        width: 100%,
        fill: surface,
        inset: 32pt,
        stroke: (left: 4pt + green),
      )[
        #text(size: 28pt, fill: rgb("#cccccc"))[
          #summary
        ]
      ]
    ] else [
      #block(
        width: 100%,
        fill: surface,
        inset: 32pt,
        stroke: (left: 4pt + green),
      )[
        #text(size: 28pt, fill: rgb("#cccccc"))[
          New #ann-type announcement from \$#symbol on the ASX. Full AI analysis available.
        ]
      ]
    ]

    #v(1fr)

    #align(center)[
      #block(
        fill: green,
        inset: (x: 48pt, y: 20pt),
        width: 100%,
      )[
        #align(center)[
          #text(size: 30pt, weight: "bold", fill: bg)[
            READ FULL ANALYSIS
          ]
        ]
      ]

      #v(32pt)

      #text(size: 32pt, weight: "bold", fill: green)[STONKWATCH]
      #v(8pt)
      #text(size: 18pt, fill: muted)[stonkwatch.app]
    ]
  ]
]
