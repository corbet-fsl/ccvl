#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Julian Y. Richard Corbet
# SPDX-License-Identifier: FSL-1.1-ALv2
# Run the Windows test stage of one exact commit on a Forgejo runner of the label
# `windows`: the executable the release run left in its draft, tested in the offline
# Windows clone that this job owns. Nothing is built, so no toolchain is provisioned.
set -euo pipefail
exec 2>&1
: "${CCID_BIN:?Missing verified ccid job runtime}"
: "${CI_COMMIT_SHA:?Missing verified source revision}"
: "${CHECKS:?Missing declared check selection}"
case "$CHECKS" in
  release-windows) ;;
  *) echo "CHECKS is not the declared Windows test: $CHECKS" >&2; exit 2 ;;
esac
export CCID_SOURCE_REVISION="$CI_COMMIT_SHA"
"$CCID_BIN" render --repo "$PWD" --check
exec "$CCID_BIN" check --repo "$PWD" --check "$CHECKS"
