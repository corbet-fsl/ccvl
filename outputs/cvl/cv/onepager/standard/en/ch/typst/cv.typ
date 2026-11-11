#import "/outputs/cvl/cv/onepager/layout.typ": render
#render(
  application-path: sys.inputs.at("application", default: "/outputs/cvl/cv/onepager/standard/en/ch/content.toml"),
  profile-path: sys.inputs.at("profile", default: "/inputs/profile.toml"),
  strings-path: sys.inputs.at("strings", default: "/outputs/cvl/cv/onepager/standard/en/ch/strings.toml"),
  substyle-path: sys.inputs.at("substyle", default: "/outputs/cvl/cv/onepager/standard/substyle.toml"),
  defaults-path: sys.inputs.at("shared-defaults", default: "/outputs/cvl/shared/onepager/defaults.toml"),
  layout-path: sys.inputs.at("layout", default: "/outputs/cvl/cv/onepager/standard/en/ch/layout.toml"),
  paper-input: sys.inputs.at("paper", default: ""),
  pages: int(sys.inputs.at("pages", default: "1")),
)
