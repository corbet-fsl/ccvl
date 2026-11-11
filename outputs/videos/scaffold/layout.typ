// Video scaffold renderer: prints only the approved profile header until a design is defined.
#import "@local/ccvl:1.0.0": load-application, load-profile, resolve-paper

// Engine helpers run inside the package; this reader resolves workspace paths here.
#let read-project(path) = read(path, encoding: none)

#let render(
  application-path: none,
  profile-path: none,
  strings-path: none,
  layout-path: none,
  paper-input: "",
  pages: 1,
) = {
  let application = load-application(application-path, read: read-project)
  let strings = toml(strings-path)
  let layout = toml(layout-path)
  let (profile, localized-profile) = load-profile(profile-path, read: read-project)
  let paper = resolve-paper(
    toml("style.toml"),
    strings.locale,
    requested: paper-input,
    recorded: application.options.at("videos_paper", default: none),
  )
  assert(pages == 1, message: "The video scaffold renders exactly one page")
  assert(application.options.language == strings.locale, message: "Video scaffold locale mismatch")
  set document(title: strings.title + " | " + profile.name, author: (profile.name,))
  set page(paper: paper.settings.page.paper, margin: 20mm)
  set text(
    font: "Archivo",
    size: 12pt,
    lang: layout.text.lang,
    region: layout.text.region,
    dir: if layout.text.direction == "rtl" { rtl } else { ltr },
  )
  text(weight: "bold", size: 24pt, profile.name)
  linebreak()
  (profile.email, profile.phone-label, profile.location, profile.website).join([ | ])
}
