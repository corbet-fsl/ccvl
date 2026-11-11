// Cluster CV, timeline, en-ch: the clustered opening page, then the unchanged
// Harvard D+ pages 2–4. Defaults also make this leaf directly compilable.
#import "/outputs/cvl/cv/cluster/layout.typ": render
#import "/outputs/cvl/cv/harvard/src/cv.typ": cv-renderer
#import "/outputs/cvl/cv/harvard/src/entries-en.typ": render-continuation
#let pages = int(sys.inputs.at("pages", default: "4"))
#let application-path = sys.inputs.at("application", default: "/outputs/cvl/cv/cluster/timeline/en/ch/content.toml")
#let profile-path = sys.inputs.at("profile", default: "/inputs/profile.toml")
#let paper-input = sys.inputs.at("paper", default: "")
#let harvard = cv-renderer(
  application-path: application-path,
  profile-path: profile-path,
  cv-pages: pages,
  strings-path: "/outputs/cvl/cv/harvard/d-plus/en/ch/strings.toml",
  substyle-path: "/outputs/cvl/cv/harvard/d-plus/substyle.toml",
  shared-defaults-path: "/outputs/cvl/shared/harvard/defaults.toml",
  layout-path: "/outputs/cvl/cv/harvard/d-plus/en/ch/layout.toml",
  paper-input: paper-input,
)
#render(
  application-path: application-path,
  profile-path: profile-path,
  strings-path: sys.inputs.at("strings", default: "/outputs/cvl/cv/cluster/timeline/en/ch/strings.toml"),
  substyle-path: sys.inputs.at("substyle", default: "/outputs/cvl/cv/cluster/timeline/substyle.toml"),
  defaults-path: sys.inputs.at("shared-defaults", default: "/outputs/cvl/shared/cluster/defaults.toml"),
  layout-path: sys.inputs.at("layout", default: "/outputs/cvl/cv/cluster/timeline/en/ch/layout.toml"),
  paper-input: paper-input,
  pages: pages,
  continuation: () => {
    (harvard.begin-continuation)()
    render-continuation(harvard)
  },
)
