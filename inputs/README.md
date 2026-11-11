# Inputs

Everything documents are made from, and nothing that is rendered as such:

```text
inputs/
├── profile.toml     the public render profile (name, contact, languages)
├── interviews/      imported sources, working profile, journal, station plan
├── assets/          binary objects used in outputs, by kind:
│   └── images/      signatures/ and, in a private workspace, avatars/,
│                    logos/, icons/ and backgrounds/
└── library/         material that is consulted but never rendered
```

In this public repository `library/` holds only the author's own or freely
licensed material; third-party material stays in a private workspace. The
paths are declared once in the `workspace_groups` table of `ccvl.json`.
