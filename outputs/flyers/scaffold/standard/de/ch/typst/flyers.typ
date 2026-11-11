#import "/outputs/flyers/scaffold/layout.typ": render
#render(
  application-path: sys.inputs.at("application", default: "/outputs/flyers/scaffold/standard/de/ch/content.toml"),
  profile-path: sys.inputs.at("profile", default: "/inputs/profile.toml"),
  strings-path: sys.inputs.at("strings", default: "/outputs/flyers/scaffold/standard/de/ch/strings.toml"),
  layout-path: sys.inputs.at("layout", default: "/outputs/flyers/scaffold/standard/de/ch/layout.toml"),
  paper-input: sys.inputs.at("paper", default: ""),
  pages: int(sys.inputs.at("pages", default: "1")),
)
