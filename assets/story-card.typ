#set page(width: 1080pt, height: 1920pt, margin: 0pt)
#set text(font: "JetBrains Mono", fill: rgb("#e0e0e0"))

#let bg = rgb("#0a0a0a")
#let surface = rgb("#141414")
#let green = rgb("#00ff88")
#let red = rgb("#ff4444")
#let amber = rgb("#ffaa00")
#let muted = rgb("#666666")
#let border = rgb("#333333")

#let symbol = sys.inputs.at("symbol", default: "ASX")
#let title = sys.inputs.at("title", default: "Announcement")
#let ann-type = sys.inputs.at("type", default: "General")
#let summary = sys.inputs.at("summary", default: "")
#let is-sensitive = sys.inputs.at("sensitive", default: "false")
#let link = sys.inputs.at("link", default: "stonkwatch.app")

#block(width: 100%, height: 100%, fill: bg)[
  #block(width: 100%, height: 6pt, fill: green)

  #pad(x: 60pt, top: 120pt)[
    #text(size: 72pt, weight: "bold", fill: green)[
      \$#symbol
    ]

    #if is-sensitive == "true" [
      #v(16pt)
      #box(
        fill: amber.transparentize(80%),
        inset: (x: 16pt, y: 10pt),
      )[
        #text(size: 24pt, weight: "bold", fill: amber)[⚡ PRICE SENSITIVE]
      ]
    ]

    #v(40pt)

    #text(size: 40pt, weight: "bold", fill: rgb("#ffffff"))[
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
    ]

    #v(1fr)

    #align(center)[
      #block(
        fill: green,
        inset: (x: 40pt, y: 16pt),
        width: 100%,
      )[
        #text(size: 28pt, weight: "bold", fill: bg)[
          SWIPE UP — FULL ANALYSIS
        ]
      ]

      #v(24pt)

      #text(size: 22pt, fill: muted)[
        #link
      ]

      #v(40pt)

      #text(size: 28pt, weight: "bold", fill: green)[STONKWATCH]
      #v(8pt)
      #text(size: 20pt, fill: muted)[ASX Market Intelligence]
    ]

    #v(80pt)
  ]
]
