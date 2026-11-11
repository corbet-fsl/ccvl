// Onepager CV renderer: one A4 page with a two-column header (name, headline,
// contact rows and summary beside an optional application photo) and optional
// skills, experience, education and project sections. Content that a leaf
// leaves empty is simply not rendered, so an empty scaffold prints the header.
#import "@local/ccvl:1.0.0": (
  apply-document-settings, load-application, load-profile, measured-content-line, measured-paragraph, paper-settings,
  resolve-paper, wrap-exact,
)

// Engine helpers run inside the package; this reader resolves workspace paths here.
#let read-project(path) = read(path, encoding: none)

#let render(
  application-path: none,
  profile-path: none,
  strings-path: none,
  substyle-path: none,
  defaults-path: none,
  layout-path: none,
  paper-input: "",
  pages: 1,
) = {
  assert(pages == 1, message: "The one-pager requires exactly one page")
  let application = load-application(application-path, read: read-project)
  let cv = application.cv
  let strings = toml(strings-path)
  let (profile, localized-profile) = load-profile(profile-path, read: read-project)
  let localized = localized-profile.at(strings.locale)
  let paper = resolve-paper(
    toml("style.toml"),
    strings.locale,
    requested: paper-input,
    recorded: application.options.at("cv_paper", default: none),
  )
  let settings = paper-settings((toml(defaults-path), toml(substyle-path), toml(layout-path)), paper)
  let design = settings.one_pager
  assert(application.options.language == strings.locale, message: "One-pager locale mismatch")
  assert(application.options.pages == 1, message: "One-pager record must select one page")
  let headline = cv.at("headline", default: "").trim()
  let summary-text = cv.at("summary", default: "").trim()
  let portrait-path = cv.at("portrait_path", default: "").trim()
  let skills = cv.at("skills", default: ())
  let experience = cv.at("experience", default: ())
  let education = cv.at("education", default: ())
  let projects = cv.at("projects", default: ())
  let ids = ()
  for entry in experience + education + projects {
    assert(entry.id not in ids, message: "Each entry must have a unique ID")
    ids.push(entry.id)
  }
  let experience-ids = experience.map(entry => entry.id)
  let grouped-ids = ()
  let groups = cv.at("experience_groups", default: ())
  for group in groups {
    assert(group.title.trim() != "", message: "An experience group needs a visible title")
    assert(group.organisations.trim() != "", message: "An experience group needs its organisations")
    assert(group.period.trim() != "", message: "An experience group needs a visible period")
    assert(group.entry_ids.len() >= 1, message: "An experience period needs at least one station")
    let positions = ()
    for id in group.entry_ids {
      assert(id in experience-ids, message: "An experience group references an unknown station")
      assert(id not in grouped-ids, message: "A station cannot occur in two experience groups")
      grouped-ids.push(id)
      positions.push(experience-ids.position(value => value == id))
    }
    assert(
      positions == range(positions.first(), positions.first() + positions.len()),
      message: "Grouped experience stations must retain their existing consecutive order",
    )
  }
  assert(grouped-ids == experience-ids, message: "Every experience station must appear once under its group, in order")
  set document(title: strings.document_title + " | " + profile.name, author: (profile.name,))
  show: apply-document-settings.with(settings)
  set text(hyphenate: false)
  show link: it => text(fill: rgb(settings.accents.link), it)

  let row(id, kind, body) = measured-content-line(
    id,
    kind,
    body,
    (
      "one-pager-headline": 40,
      "one-pager-section": 8,
      "one-pager-heading": 8,
      "one-pager-role": 35,
      "one-pager-bullet": 65,
      "one-pager-skill": 75,
      "one-pager-period": 8,
      "one-pager-group": 8,
    ).at(kind),
    90,
    100,
  )
  let rule() = block(
    width: 100%,
    height: design.rule_width_pt * 1pt,
    fill: rgb(design.rule_color),
    above: 0pt,
    below: 0pt,
  )
  // Every section heading sits between two rules: rule, heading, rule.
  let section(title, id) = block(width: 100%, breakable: false, sticky: true)[
    #v(design.section_before_pt * 1pt)
    #move(dy: design.heading_shift_pt * 1pt)[
      #rule()
      #v(design.rule_gap_pt * 1pt)
      #row(id, "one-pager-section", text(
        size: design.section_size_pt * 1pt,
        weight: "bold",
        fill: rgb(design.section_color),
        title,
      ))
      #v(design.rule_gap_pt * 1pt)
      #rule()
    ]
    #v(design.section_after_pt * 1pt)
  ]
  let bullet-mark = box(width: 10.5pt, height: 7.35pt, align(horizon, align(center, polygon(
    fill: black,
    (0pt, 0pt),
    (4.41pt, 2.75625pt),
    (0pt, 5.5125pt),
  ))))
  let bullet-row(entry) = {
    let body = entry.bullet
    let emphasis = entry.at("bullet_emphasis", default: "")
    if emphasis != "" {
      let parts = entry.bullet.split(emphasis)
      assert(parts.len() == 2, message: "Bullet emphasis must match exactly once: " + entry.id)
      body = parts.first() + text(weight: "bold", emphasis) + parts.last()
    }
    grid(
      columns: (design.bullet_indent_pt * 1pt, 1fr),
      gutter: 0pt,
      bullet-mark, row(entry.id + ".bullet", "one-pager-bullet", body),
    )
  }
  // Eraneos-style line: the organisation part through the separator in the
  // label weight, followed by the role or period in regular weight.
  let labelled(label, rest) = text(
    size: design.station_size_pt * 1pt,
    text(weight: design.station_label_weight, label + " |") + " " + rest,
  )
  // Experience stations: an optional "organisation, place | role" heading and
  // one bullet. Stations without a heading continue the group's bullet list.
  let stations(values) = {
    for (index, entry) in values.enumerate() {
      let heading = entry.at("heading", default: "")
      if index > 0 {
        v((if heading == "" { design.bullet_list_gap_pt } else { design.subentry_gap_pt }) * 1pt)
      }
      block(width: 100%, breakable: false)[
        #if heading != "" {
          let parts = heading.split(" | ")
          assert(parts.len() == 2, message: "A station heading needs one ' | ' separator: " + entry.id)
          row(entry.id + ".heading", "one-pager-heading", labelled(parts.first(), parts.last()))
          v(design.station_bullet_gap_pt * 1pt)
        }
        #bullet-row(entry)
      ]
    }
  }
  let entries(values, with-bullet: true, gap: design.entry_gap_pt) = {
    for (index, entry) in values.enumerate() {
      if index > 0 { v(gap * 1pt) }
      block(width: 100%, breakable: false)[
        #let heading = text(
          size: design.heading_size_pt * 1pt,
          weight: design.heading_weight,
          entry.heading,
        )
        #if entry.at("url", default: "") != "" {
          heading += text(size: design.project_link_size_pt * 1pt)[
            #h(6pt)#link(entry.url, entry.link_label)
          ]
        }
        #row(entry.id + ".heading", "one-pager-heading", heading)
        #v(design.role_gap_pt * 1pt)
        #row(entry.id + ".role", "one-pager-role", text(
          size: design.role_size_pt * 1pt,
          fill: rgb(design.secondary_color),
          entry.role,
        ))
        #if with-bullet {
          v(design.bullet_gap_pt * 1pt)
          bullet-row(entry)
        }
      ]
    }
  }
  let primary-contacts = (
    link("mailto:" + profile.email)[#profile.email],
    if profile.phone-label != none and profile.phone-href != none { link(profile.phone-href)[#profile.phone-label] },
    if cv.at("location", default: "") == "" { profile.location } else { cv.location },
    link(profile.linkedin)[LinkedIn],
    link(profile.website)[Web],
  ).filter(item => item != none)
  let secondary-contacts = (
    if cv.at("languages", default: "") == "" { profile.languages } else { cv.languages },
    localized.nationality-and-permit,
    localized.availability,
  ).filter(item => item != none)
  let contact-row(items, fill: black) = text(
    size: design.contact_size_pt * 1pt,
    fill: fill,
    box(items.join(
      h(design.contact_separator_gap_pt * 1pt)
        + text(fill: rgb(design.secondary_color), "|")
        + h(design.contact_separator_gap_pt * 1pt),
    )),
  )
  // Two-column header without a profile heading: name, headline, contact rows
  // and the summary on the left; the portrait on the right keeps the classic
  // application-photo ratio and spans exactly from the top of the name to the
  // baseline of the last summary line.
  let gap = design.portrait_gap_mm * 1mm
  let count = design.summary_lines
  let text-column(width, summary) = block(width: width, breakable: false)[
    #text(
      size: design.name_size_pt * 1pt,
      weight: "bold",
      fill: rgb(design.name_color),
      top-edge: "bounds",
      profile.name,
    )
    #if headline != "" {
      v(design.headline_gap_pt * 1pt)
      row("headline", "one-pager-headline", text(
        size: design.headline_size_pt * 1pt,
        weight: "medium",
        fill: rgb(design.secondary_color),
        headline,
      ))
    }
    #v(settings.header.after_pt * 1pt)
    #contact-row(primary-contacts)
    #v(design.contact_gap_pt * 1pt)
    #contact-row(
      secondary-contacts,
      fill: rgb(design.secondary_color),
    )
    #if summary != none {
      v(design.summary_before_pt * 1pt)
      summary
    }
  ]
  let summary-paragraph(width) = {
    if summary-text == "" { return none }
    let lines = wrap-exact(summary-text, width, count, "one-pager.summary", last-max: 100)
    measured-paragraph(
      "one-pager.summary",
      "cv-summary",
      lines
        .enumerate()
        .map(((i, value)) => (
          text: value,
          min_fill: if cv.at("allow_thin", default: false) { 1 } else { 95 },
          target_fill: 97,
          max_fill: 100,
        )),
      justify: false,
    )
  }
  layout(size => {
    if portrait-path == "" {
      text-column(size.width, summary-paragraph(size.width))
    } else {
      // The height depends only on the fixed line count, so measure it with
      // placeholder lines first, then derive the portrait width from its ratio.
      let placeholder = if summary-text == "" { none } else { range(count).map(_ => "Hg").join(linebreak()) }
      let height = measure(text-column(size.width, placeholder)).height
      let portrait-width = height * design.portrait_ratio
      let column = size.width - portrait-width - gap
      grid(
        columns: (column, portrait-width),
        column-gutter: gap,
        align: (left + top, left + top),
        text-column(column, summary-paragraph(column)),
        image(portrait-path, width: portrait-width, height: height, fit: "cover", alt: profile.name),
      )
    }
  })
  if skills.len() > 0 { section(strings.skills, "skills") }
  for (index, skill) in skills.enumerate() {
    if index > 0 { v(design.skill_gap_pt * 1pt) }
    let parts = skill.split(":")
    let body = if parts.len() > 1 {
      text(weight: "bold", parts.first() + ":") + parts.slice(1).join(":")
    } else { skill }
    row("skill." + str(index + 1), "one-pager-skill", body)
  }
  if groups.len() > 0 { section(strings.experience, "experience") }
  for (index, group) in groups.enumerate() {
    if index > 0 { v(design.entry_gap_pt * 1pt) }
    // Group: title, then "organisations | period", then its stations.
    block(width: 100%, breakable: false)[
      #row("experience.group." + str(index) + ".title", "one-pager-period", text(
        size: design.heading_size_pt * 1pt,
        weight: "bold",
        fill: rgb(design.group_title_color),
        group.title,
      ))
      #v(design.role_gap_pt * 1pt)
      #row("experience.group." + str(index) + ".line", "one-pager-group", labelled(group.organisations, group.period))
      #v(design.group_after_pt * 1pt)
      #block(width: 100%, inset: (left: design.subentry_inset_mm * 1mm))[
        #stations(group.entry_ids.map(id => experience.find(entry => entry.id == id)))
      ]
    ]
  }
  if education.len() > 0 {
    section(strings.education, "education")
    entries(education, with-bullet: false, gap: design.education_gap_pt)
  }
  if projects.len() > 0 {
    section(strings.projects, "projects")
    entries(projects)
  }
  context assert(
    counter(page).final().first() == 1,
    message: "One-pager overflow: edit the selection; keep the type sizes",
  )
}
