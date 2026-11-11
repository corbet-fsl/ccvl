#import "/outputs/videos/scaffold/layout.typ": render
#render(
  application-path: sys.inputs.at("application", default: "/outputs/videos/scaffold/standard/en/ch/content.toml"),
  profile-path: sys.inputs.at("profile", default: "/inputs/profile.toml"),
  strings-path: sys.inputs.at("strings", default: "/outputs/videos/scaffold/standard/en/ch/strings.toml"),
  layout-path: sys.inputs.at("layout", default: "/outputs/videos/scaffold/standard/en/ch/layout.toml"),
  paper-input: sys.inputs.at("paper", default: ""),
  pages: int(sys.inputs.at("pages", default: "1")),
)
