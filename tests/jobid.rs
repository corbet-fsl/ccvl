//! The job id language under the `jobid` feature alone: golden values of `@cv/jobcache` `ids.ts`.
#![cfg(feature = "jobid")]

use ccvl::url_rules::{
    ALLOW_NAMES, BASE64_MIN_LEN, HOST_PARAMS, NAME_CONTAINS, NAME_EQUALS, NAME_PREFIXES,
    NAME_SUFFIXES, QUERY_SEPARATORS,
};
use ccvl::{URL_RULE_VERSION, contains_email, is_personal_url, job_id_from_url, normalize_url};

const URL: &str = "https://EXAMPLE.com/jobs/42/?utm_source=news#top";

#[test]
fn normalizes_like_typescript() {
    assert_eq!(normalize_url(URL).unwrap(), "https://example.com/jobs/42");
}

#[test]
fn job_id_matches_typescript_golden_vector() {
    assert_eq!(job_id_from_url(URL).unwrap(), "job_ttiaykbb4hwpf7ff");
}

fn strings(v: &serde_json::Value) -> Vec<String> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_str().unwrap().to_string())
        .collect()
}

fn owned(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| (*s).to_string()).collect()
}

/// The shared fixture (also read by `@cv/jobcache/ids`): every row byte for byte, ids and personal flags included, and the lists.
#[test]
fn shared_fixture_rows_and_lists_match_the_typescript_twin() {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/normalize-url.json")).unwrap();
    assert_eq!(fixture["rule_version"], URL_RULE_VERSION);
    assert_eq!(URL_RULE_VERSION, 3);
    let rules = &fixture["rules"];
    assert_eq!(
        strings(&rules["query_separators"]),
        QUERY_SEPARATORS
            .iter()
            .map(char::to_string)
            .collect::<Vec<_>>()
    );
    assert_eq!(strings(&rules["name_contains"]), owned(NAME_CONTAINS));
    assert_eq!(strings(&rules["name_equals"]), owned(NAME_EQUALS));
    assert_eq!(strings(&rules["name_suffixes"]), owned(NAME_SUFFIXES));
    assert_eq!(strings(&rules["name_prefixes"]), owned(NAME_PREFIXES));
    assert_eq!(strings(&rules["allow_names"]), owned(ALLOW_NAMES));
    assert_eq!(
        usize::try_from(rules["base64_min_len"].as_u64().unwrap()).unwrap(),
        BASE64_MIN_LEN
    );
    let hosts = rules["host_params"].as_array().unwrap();
    assert_eq!(hosts.len(), HOST_PARAMS.len());
    for (h, (pattern, names)) in hosts.iter().zip(HOST_PARAMS) {
        assert_eq!(h["host"].as_str().unwrap(), *pattern);
        assert_eq!(strings(&h["names"]), owned(names));
    }
    let cases = fixture["cases"].as_array().unwrap();
    assert!(cases.len() >= 75);
    for c in cases {
        let (name, input) = (c["name"].as_str().unwrap(), c["input"].as_str().unwrap());
        assert_eq!(
            normalize_url(input).unwrap(),
            c["normalized"].as_str().unwrap(),
            "{name}"
        );
        assert_eq!(
            job_id_from_url(input).unwrap(),
            c["job_id"].as_str().unwrap(),
            "{name}"
        );
        assert_eq!(
            is_personal_url(input).unwrap(),
            c["personal"].as_bool().unwrap(),
            "{name}"
        );
    }
}

#[test]
fn hand_written_expectations() {
    let b = "https://jobs.example.com/p/42";
    assert_eq!(
        normalize_url(&format!("{b}?e=alice@example.com&id=7")).unwrap(),
        format!("{b}?id=7")
    );
    assert_eq!(
        normalize_url(&format!("{b}?mail=alice%40example.com&id=7")).unwrap(),
        format!("{b}?id=7")
    );
    assert_eq!(normalize_url(&format!("{b}?contact=a@b.co")).unwrap(), b);
    assert_eq!(
        normalize_url(&format!("{b}?jk=alice@example.com&id=1")).unwrap(),
        format!("{b}?id=1")
    ); // rule 1 beats the allow-list
    assert_eq!(
        normalize_url(&format!(
            "{b}?otpToken=1&midSig=2&emailhash=3&user_token=4&id_token=5&auth_token=6&id=1"
        ))
        .unwrap(),
        format!("{b}?id=1")
    );
    assert_eq!(
        normalize_url(&format!("{b}?a=1;token=x;b=2")).unwrap(),
        format!("{b}?a=1&b=2")
    );
    assert_eq!(
        normalize_url(&format!("{b}?pass=1&pwd=2&passwd=3&id=1")).unwrap(),
        format!("{b}?id=1")
    );
    // `;` splits only before a name= token: in a value, before an empty name, in an entity it does not
    assert_eq!(
        normalize_url(&format!("{b}?x=a;b&id=1")).unwrap(),
        format!("{b}?id=1&x=a%3Bb")
    );
    assert_eq!(
        normalize_url(&format!("{b}?q=it&apos;s&id=1")).unwrap(),
        format!("{b}?apos%3Bs=&id=1&q=it")
    );
    assert_eq!(
        normalize_url(&format!("{b}?x=1;=2&id=1")).unwrap(),
        format!("{b}?id=1&x=1%3B%3D2")
    );
    assert_eq!(
        normalize_url("https://jobs.example.com/job/123;final/x").unwrap(),
        "https://jobs.example.com/job/123;final/x"
    );
    assert!(is_personal_url("https://user:pass@jobs.example.com/p/1").unwrap());
    assert!(is_personal_url("https://user@jobs.example.com/p/1").unwrap());
    assert!(!is_personal_url(&format!("{b}?id=1")).unwrap());
    assert_eq!(
        normalize_url("https://jobs.example.com/job/123;jsessionid=ABC").unwrap(),
        "https://jobs.example.com/job/123"
    );
    assert_eq!(
        normalize_url("https://jobs.example.com/job/123;jk=77").unwrap(),
        "https://jobs.example.com/job/123;jk=77"
    );
    assert_eq!(
        normalize_url("https://ch.indeed.com/viewjob?jk=abc&alid=1&qd=2&rd=3").unwrap(),
        "https://ch.indeed.com/viewjob?jk=abc"
    );
    assert_eq!(
        normalize_url(
            "https://www.linkedin.com/jobs/view/1?eid=a&refId=b&trackingId=c&currentJobId=1"
        )
        .unwrap(),
        "https://www.linkedin.com/jobs/view/1?currentJobId=1"
    );
    assert_eq!(
        normalize_url(&format!("{b}?state=YWxpY2VAZXhhbXBsZS5jb20=&id=1")).unwrap(),
        format!("{b}?id=1")
    );
}

/// NEGATIVE CONTROLS: these stay.
#[test]
fn allow_list_corpus_names_and_lookalikes_are_kept() {
    let b = "https://jobs.example.com/p/42";
    for name in [
        "key",
        "id",
        "uid",
        "sid",
        "jk",
        "vjk",
        "jid",
        "currentJobId",
        "jobId",
        "gh_jid",
        "jobListingId",
    ] {
        assert_eq!(
            normalize_url(&format!("{b}?{name}=abc123")).unwrap(),
            format!("{b}?{name}=abc123"),
            "{name}"
        );
    }
    for name in [
        "gh_jid",
        "id",
        "jobDbPVId",
        "l",
        "lang",
        "sid",
        "filter",
        "application",
        "q",
        "sortColumn",
        "sortDirection",
        "language",
        "mw_source",
        "locationsearch",
    ] {
        assert_eq!(
            normalize_url(&format!("{b}?{name}=v1")).unwrap(),
            format!("{b}?{name}=v1"),
            "{name}"
        );
    }
    assert_eq!(
        normalize_url(&format!(
            "{b}?passport=1&pwdx=2&design=3&hashtag=4&option=5&author=6&id=1"
        ))
        .unwrap(),
        format!("{b}?author=6&design=3&hashtag=4&id=1&option=5&passport=1&pwdx=2")
    );
    assert_eq!(
        normalize_url(&format!("{b}?q=a@b&q2=@example.com&q3=a@b.c&id=1")).unwrap(),
        format!("{b}?id=1&q=a%40b&q2=%40example.com&q3=a%40b.c")
    );
    assert_eq!(
        normalize_url(&format!("{b}?eid=a&lk=b&alid=c")).unwrap(),
        format!("{b}?alid=c&eid=a&lk=b")
    );
}

#[test]
fn email_detection() {
    for ok in [
        "a@b.co",
        "write to bob@corp.io today",
        "alice.b+jobs@Example.CO.UK",
        "j\u{fc}rgen@b\u{fc}cher.de",
        "a@b.co.",
    ] {
        assert!(contains_email(ok), "{ok}");
    }
    for no in [
        "", "a@b", "@b.co", "a@.co", "a@b.c", "a@b.12", "plain", "a@",
    ] {
        assert!(!contains_email(no), "{no:?}");
    }
}

#[test]
fn glassdoor_and_jobs_ch_alert_parameters_are_dropped() {
    assert_eq!(
        normalize_url(
            "https://www.glassdoor.com/job-listing/x?jl=1&guid=a&jrtk=b&jobListingId=1234"
        )
        .unwrap(),
        "https://www.glassdoor.com/job-listing/x?jl=1&jobListingId=1234"
    );
    assert_eq!(
        normalize_url("https://www.jobs.ch/en/vacancies/detail/abc/?jobalert_id=77&id=1").unwrap(),
        "https://www.jobs.ch/en/vacancies/detail/abc?id=1"
    );
    // NEGATIVE CONTROL: another host that merely ends like jobs.ch keeps the name
    assert_eq!(
        normalize_url("https://notjobs.ch/v/1?jobalert_id=5").unwrap(),
        "https://notjobs.ch/v/1?jobalert_id=5"
    );
}

#[test]
fn many_parameters_normalize_in_linear_time() {
    let pairs: Vec<String> = (0..20_000u64)
        .map(|i| format!("p{}=v{}", (i * 7919) % 20_000, i))
        .collect();
    let url = format!("https://jobs.example.com/p/1?{}&id=1", pairs.join("&"));
    let t0 = std::time::Instant::now();
    let out = normalize_url(&url).unwrap();
    // 1.2 s before the sort-and-filter rewrite; well under a second now (the bound is loose for a busy machine)
    assert!(
        t0.elapsed() < std::time::Duration::from_secs(5),
        "{:?}",
        t0.elapsed()
    );
    assert!(out.starts_with("https://jobs.example.com/p/1?id=1&p0="));
    let long = format!(
        "https://jobs.example.com/p/1?x={}{}&id=1",
        "A".repeat(16),
        "=".repeat(100_000)
    );
    let t1 = std::time::Instant::now();
    normalize_url(&long).unwrap();
    assert!(
        t1.elapsed() < std::time::Duration::from_secs(5),
        "{:?}",
        t1.elapsed()
    );
}

/// The stable sort and filter keeps the order of the old key-by-key grouping: checked against a naive reference.
#[test]
fn sort_and_filter_matches_the_naive_grouping() {
    let names = [
        "a",
        "b",
        "B",
        "z",
        "\u{e9}",
        "\u{1F600}",
        "\u{FF21}",
        "utm_source",
        "token",
        "id",
        "q",
    ];
    let mut seed: u32 = 7;
    let mut rnd = || {
        seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        seed
    };
    for _ in 0..50 {
        let n = 1 + (rnd() % 12) as usize;
        let q: Vec<String> = (0..n)
            .map(|_| format!("{}={}", names[(rnd() % 11) as usize], rnd() % 5))
            .collect();
        let url = format!("https://jobs.example.com/p/1?{}", q.join("&"));
        let parsed = url::Url::parse(&url).unwrap();
        let pairs: Vec<(String, String)> = parsed
            .query_pairs()
            .map(|(k, v)| (k.into_owned(), v.into_owned()))
            .collect();
        let mut keys: Vec<&String> = Vec::new();
        for (k, _) in &pairs {
            if !keys.contains(&k) {
                keys.push(k);
            }
        }
        keys.sort_by(|a, b| a.encode_utf16().cmp(b.encode_utf16()));
        let mut want: Vec<(String, String)> = Vec::new();
        for k in keys {
            if ["utm_source", "token"].contains(&k.to_lowercase().as_str()) {
                continue;
            }
            for (pk, pv) in &pairs {
                if pk == k {
                    want.push((pk.clone(), pv.clone()));
                }
            }
        }
        let got = url::Url::parse(&normalize_url(&url).unwrap()).unwrap();
        let got: Vec<(String, String)> = got
            .query_pairs()
            .map(|(k, v)| (k.into_owned(), v.into_owned()))
            .collect();
        assert_eq!(got, want, "{url}");
    }
}
