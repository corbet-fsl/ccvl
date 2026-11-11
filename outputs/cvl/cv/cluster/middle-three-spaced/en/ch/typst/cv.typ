// Cluster CV, middle-three-spaced, en-ch: the clustered opening page, then the unchanged
// Harvard D+ pages 2–4. Defaults also make this leaf directly compilable.
#import "/cvl/cv/cluster/layout.typ": render
#import "/cvl/cv/harvard/src/cv.typ": cv-renderer
#import "/cvl/cv/harvard/src/entries-en.typ": render-continuation
#let pages = int(sys.inputs.at("pages", default: "4"))
#let application-path = sys.inputs.at("application", default: "/cvl/cv/cluster/middle-three-spaced/en/ch/content.toml")
#let profile-path = sys.inputs.at("profile", default: "/cvl/profile.toml")
#let paper-input = sys.inputs.at("paper", default: "")
#let harvard = cv-renderer(
  application-path: application-path,
  profile-path: profile-path,
  cv-pages: pages,
  strings-path: "/cvl/cv/harvard/d-plus/en/ch/strings.toml",
  substyle-path: "/cvl/cv/harvard/d-plus/substyle.toml",
  shared-defaults-path: "/cvl/shared/harvard/defaults.toml",
  layout-path: "/cvl/cv/harvard/d-plus/en/ch/layout.toml",
  paper-input: paper-input,
)
#render(
  application-path: application-path,
  profile-path: profile-path,
  strings-path: sys.inputs.at("strings", default: "/cvl/cv/cluster/middle-three-spaced/en/ch/strings.toml"),
  substyle-path: sys.inputs.at("substyle", default: "/cvl/cv/cluster/middle-three-spaced/substyle.toml"),
  defaults-path: sys.inputs.at("shared-defaults", default: "/cvl/shared/cluster/defaults.toml"),
  layout-path: sys.inputs.at("layout", default: "/cvl/cv/cluster/middle-three-spaced/en/ch/layout.toml"),
  paper-input: paper-input,
  pages: pages,
  continuation: () => {
    (harvard.begin-continuation)()
    render-continuation(harvard)
  },
)
