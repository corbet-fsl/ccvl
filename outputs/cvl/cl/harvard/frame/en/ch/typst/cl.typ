// Harvard cover letter: frame, en-ch. Defaults also support direct compilation.
#import "/outputs/cvl/cl/harvard/src/cl.typ": render-cl
#let application-path = sys.inputs.at("application", default: "/outputs/cvl/cl/harvard/frame/en/ch/content.toml")
#let profile-path = sys.inputs.at("profile", default: "/inputs/profile.toml")
#let strings-path = sys.inputs.at("strings", default: "/outputs/cvl/cl/harvard/frame/en/ch/strings.toml")
#let substyle-path = sys.inputs.at("substyle", default: "/outputs/cvl/cl/harvard/frame/substyle.toml")
#let paper-input = sys.inputs.at("paper", default: "")
#let layout-path = sys.inputs.at("layout", default: "/outputs/cvl/cl/harvard/frame/en/ch/layout.toml")
#let shared-defaults-path = sys.inputs.at("shared-defaults", default: "/outputs/cvl/shared/harvard/defaults.toml")
#render-cl(
  application-path: application-path,
  profile-path: profile-path,
  strings-path: strings-path,
  substyle-path: substyle-path,
  shared-defaults-path: shared-defaults-path,
  layout-path: layout-path,
  paper-input: paper-input,
)
