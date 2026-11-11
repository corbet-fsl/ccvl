// Workspace assets by logical name. `asset("images/signatures/signature.png")`
// resolves against the assets folder that the engine passes as the `assets`
// input (the `inputs.assets` path of ccvl.json). The default serves direct
// compilation and must equal that manifest path; a Rust test enforces it. A
// name that starts with "/" is already a workspace path and passes unchanged.
#let asset(name) = if name.starts-with("/") { name } else {
  sys.inputs.at("assets", default: "/inputs/assets") + "/" + name
}
