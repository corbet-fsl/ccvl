// Onepager CV renderer. Scaffold only: renders the approved profile header until the design is defined.
#import "@local/ccvl:1.0.0": apply-document-settings, load-application, load-profile, paper-settings, resolve-paper

// Engine helpers run inside the package; this reader resolves workspace paths here.
#let read-project(path) = read(path, encoding: none)

#let render(
  application-path: none,
  profile-path: none,
  strings-path: none,
  substyle-path: none,
  defaults-path: none,
  layout-path: none,
  paper-input: "",
  pages: 1,
) = {
  let application = load-application(application-path, read: read-project)
  let strings = toml(strings-path)
  let (profile, localized-profile) = load-profile(profile-path, read: read-project)
  let localized = localized-profile.at(strings.locale)
  let paper = resolve-paper(
    toml("style.toml"),
    strings.locale,
    requested: paper-input,
    recorded: application.options.at("cv_paper", default: none),
  )
  let settings = paper-settings((toml(defaults-path), toml(substyle-path), toml(layout-path)), paper)
  assert(pages == 1, message: "Onepager requests exactly one page")
  assert(application.options.language == strings.locale, message: "Onepager locale mismatch")
  set document(title: "One-page CV | " + profile.name, author: (profile.name,))
  show: apply-document-settings.with(settings)

  let contacts = (
    profile.email,
    profile.phone-label,
    profile.location,
    profile.languages,
    localized.nationality-and-permit,
    localized.availability,
  ).filter(item => item != none)
  text(weight: "bold", profile.name)
  linebreak()
  contacts.join([ | ])
}
