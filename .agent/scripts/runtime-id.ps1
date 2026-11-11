# Keep the identity algorithm aligned with runtime_source.rs and runtime-id.sh.
function Get-SourceFingerprint {
    if ($env:CCVL_BOOTSTRAP_TESTING -eq "1") {
        if (Test-Path Env:CCVL_BOOTSTRAP_TEST_FINGERPRINT) {
            return $env:CCVL_BOOTSTRAP_TEST_FINGERPRINT
        }
        return "test-fingerprint"
    }
    $Files = @(
        Get-Item -ErrorAction Stop -LiteralPath (Join-Path $RepoRoot "Cargo.toml")
        Get-Item -ErrorAction Stop -LiteralPath (Join-Path $RepoRoot ".agent/core/Cargo.toml")
        Get-Item -ErrorAction Stop -LiteralPath (Join-Path $RepoRoot "Cargo.lock")
        Get-Item -ErrorAction Stop -LiteralPath (Join-Path $RepoRoot "rust-toolchain.toml")
        Get-Item -ErrorAction Stop -LiteralPath (Join-Path $RepoRoot ".agent/build.rs")
        Get-Item -ErrorAction Stop -LiteralPath (Join-Path $RepoRoot "ccvl.json")
    )
    # Rust sources and the embedded engine Typst package, sorted by the byte
    # order of their relative paths like runtime_source.rs.
    $Sources = @(
        Get-ChildItem -ErrorAction Stop -LiteralPath @((Join-Path $RepoRoot ".agent\src"), (Join-Path $RepoRoot ".agent\core\src")) -Recurse -File -Filter "*.rs"
        Get-ChildItem -ErrorAction Stop -LiteralPath (Join-Path $RepoRoot ".agent\typst") -File |
            Where-Object { $_.Extension -cin @(".typ", ".toml", ".json") }
    )
    $Relatives = { process { $_.FullName.Substring($RepoRoot.Length + 1).Replace("\", "/") } }
    $Ordered = [string[]]@($Sources | & $Relatives)
    [Array]::Sort($Ordered, [StringComparer]::Ordinal)
    $Lines = foreach ($Relative in @($Files | & $Relatives) + $Ordered) {
        $Hash = (Get-FileHash -ErrorAction Stop -Algorithm SHA256 -LiteralPath (Join-Path $RepoRoot $Relative)).Hash.ToLowerInvariant()
        "$Relative $Hash"
    }
    $Bytes = [Text.UTF8Encoding]::new($false).GetBytes(($Lines -join "`n") + "`n")
    $Hasher = [Security.Cryptography.SHA256]::Create()
    try {
        return ([BitConverter]::ToString($Hasher.ComputeHash($Bytes))).Replace("-", "").ToLowerInvariant()
    }
    finally {
        $Hasher.Dispose()
    }
}
