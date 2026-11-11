// Synthetic ledger cv leaf: compact, en-ch. Defaults support direct compilation.
#import "/outputs/cvl/shared/ledger/render.typ": render-cv
#render-cv(
  application-path: sys.inputs.at("application", default: "/outputs/cvl/cv/ledger/compact/en/ch/content.toml"),
  profile-path: sys.inputs.at("profile", default: "/inputs/profile.toml"),
  strings-path: sys.inputs.at("strings", default: "/outputs/cvl/cv/ledger/compact/en/ch/strings.toml"),
  substyle-path: sys.inputs.at("substyle", default: "/outputs/cvl/cv/ledger/compact/substyle.toml"),
  shared-defaults-path: sys.inputs.at("shared-defaults", default: "/outputs/cvl/shared/ledger/defaults.toml"),
  layout-path: sys.inputs.at("layout", default: "/outputs/cvl/cv/ledger/compact/en/ch/layout.toml"),
  paper-input: sys.inputs.at("paper", default: ""),
  pages: int(sys.inputs.at("pages", default: "4")),
)
