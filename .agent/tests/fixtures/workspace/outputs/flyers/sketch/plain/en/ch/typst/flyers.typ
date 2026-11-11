// Synthetic flyer leaf: en-ch. Defaults support direct compilation.
#let record = toml(sys.inputs.at("application", default: "/outputs/flyers/sketch/plain/en/ch/content.toml"))
#let profile = toml(sys.inputs.at("profile", default: "/inputs/profile.toml"))
#let layout = toml(sys.inputs.at("layout", default: "/outputs/flyers/sketch/plain/en/ch/layout.toml"))
#assert(record.options.language == "en-ch", message: "synthetic flyer locale mismatch")
#set page(paper: "a6", margin: 10mm)
#set text(font: "Archivo", lang: layout.text.lang, region: layout.text.region)
#profile.name \
#profile.email \
#record.flyers.tagline
