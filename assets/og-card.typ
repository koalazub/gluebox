#set page(width: 1200pt, height: 630pt, margin: 0pt)
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
#let sentiment = sys.inputs.at("sentiment", default: "neutral")
#let summary = sys.inputs.at("summary", default: "")
#let is-sensitive = sys.inputs.at("sensitive", default: "false")

#let sentiment-color = if sentiment == "positive" or sentiment == "very_positive" { green } else if sentiment == "negative" or sentiment == "very_negative" { red } else { muted }
#let sentiment-label = if sentiment == "positive" or sentiment == "very_positive" { "BULLISH" } else if sentiment == "negative" or sentiment == "very_negative" { "BEARISH" } else { "NEUTRAL" }

#block(width: 100%, height: 100%, fill: bg)[
  // Top border accent
  #block(width: 100%, height: 4pt, fill: green)

  #pad(x: 60pt, y: 40pt)[
    // Header: symbol + type
    #grid(
      columns: (auto, 1fr, auto),
      gutter: 20pt,
      [
        #text(size: 48pt, weight: "bold", fill: green)[
          \$#symbol
        ]
        #if is-sensitive == "true" [
          #h(12pt)
          #box(
            fill: amber.transparentize(80%),
            inset: (x: 12pt, y: 6pt),
          )[
            #text(size: 16pt, weight: "bold", fill: amber)[⚡ PRICE SENSITIVE]
          ]
        ]
      ],
      [],
      [
        #box(
          fill: surface,
          inset: (x: 16pt, y: 10pt),
          stroke: 1pt + border,
        )[
          #text(size: 14pt, fill: muted)[#ann-type]
        ]
      ],
    )

    #v(24pt)

    // Title
    #text(size: 28pt, weight: "bold", fill: rgb("#ffffff"))[
      #title
    ]

    #v(20pt)

    // Sentiment badge
    #box(
      fill: sentiment-color.transparentize(85%),
      inset: (x: 16pt, y: 8pt),
      stroke: 1pt + sentiment-color.transparentize(50%),
    )[
      #text(size: 16pt, weight: "bold", fill: sentiment-color)[
        #sentiment-label
      ]
    ]

    #v(20pt)

    // Summary preview
    #if summary != "" [
      #block(
        width: 100%,
        fill: surface,
        inset: 24pt,
        stroke: (left: 3pt + green),
      )[
        #text(size: 18pt, fill: rgb("#cccccc"))[
          #summary
        ]
      ]
    ]

    // Footer
    #v(1fr)
    #grid(
      columns: (1fr, auto),
      [
        #text(size: 20pt, weight: "bold", fill: green)[STONKWATCH]
        #h(8pt)
        #text(size: 16pt, fill: muted)[ASX Market Intelligence]
      ],
      [
        #text(size: 14pt, fill: muted)[stonkwatch.app]
      ],
    )
  ]
]
