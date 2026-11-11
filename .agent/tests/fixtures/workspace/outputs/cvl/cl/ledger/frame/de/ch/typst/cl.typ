// Synthetic ledger cl leaf: frame, de-ch. Defaults support direct compilation.
#import "/outputs/cvl/shared/ledger/render.typ": render-cl
#render-cl(
  application-path: sys.inputs.at("application", default: "/outputs/cvl/cl/ledger/frame/de/ch/content.toml"),
  profile-path: sys.inputs.at("profile", default: "/inputs/profile.toml"),
  strings-path: sys.inputs.at("strings", default: "/outputs/cvl/cl/ledger/frame/de/ch/strings.toml"),
  substyle-path: sys.inputs.at("substyle", default: "/outputs/cvl/cl/ledger/frame/substyle.toml"),
  shared-defaults-path: sys.inputs.at("shared-defaults", default: "/outputs/cvl/shared/ledger/defaults.toml"),
  layout-path: sys.inputs.at("layout", default: "/outputs/cvl/cl/ledger/frame/de/ch/layout.toml"),
  paper-input: sys.inputs.at("paper", default: ""),
)
