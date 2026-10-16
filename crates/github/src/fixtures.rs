//! Recorded `gh` output (a made-up repository, acme/site) for tests.

/// 2026-10-16 15:00 UTC.
pub const NOW: i64 = 1_792_162_800_000;

pub const REPO: &str = r#"{"nameWithOwner":"acme/site"}"#;

pub const ISSUES: &str = r#"[
 {"number":12,"title":"Header jumps when the menu opens","state":"OPEN","author":{"login":"alice","is_bot":false},
  "labels":[{"name":"bug","color":"d73a4a"},{"name":"ui","color":"0075ca"}],
  "comments":[{"id":"a","body":"first"},{"id":"b","body":"second"},{"id":"c","body":"third"}],
  "updatedAt":"2026-10-16T12:00:00Z"},
 {"number":9,"title":"Dark mode contrast is too low on the settings page, especially in the sidebar","state":"OPEN","author":{"login":"app/dependabot","is_bot":true},
  "labels":[],"comments":[],"updatedAt":"2026-10-14T15:00:00Z"},
 {"number":4,"title":"Write a contributing guide","state":"CLOSED","author":{"login":"carol"},
  "labels":[{"name":"docs"}],"comments":[{"id":"d","body":"done"}],"updatedAt":"2026-09-01T15:00:00Z"}
]"#;

pub const PRS: &str = r#"[
 {"number":31,"title":"Fix the sticky header","state":"OPEN","isDraft":false,"author":{"login":"bob"},
  "labels":[{"name":"ui"}],"comments":[{"id":"a","body":"x"}],"updatedAt":"2026-10-16T14:30:00Z"},
 {"number":30,"title":"WIP: new theme","state":"OPEN","isDraft":true,"author":{"login":"alice"},
  "labels":[],"comments":[],"updatedAt":"2026-10-15T15:00:00Z"},
 {"number":28,"title":"Bump the toolchain","state":"MERGED","isDraft":false,"author":{"login":"app/dependabot"},
  "labels":[{"name":"dependencies"},{"name":"rust"}],"comments":[],"updatedAt":"2026-10-09T15:00:00Z"},
 {"number":25,"title":"Try tabs","state":"CLOSED","isDraft":false,"author":{"login":"dave"},
  "labels":[],"comments":[],"updatedAt":"2026-08-01T15:00:00Z"}
]"#;

pub const ISSUE_12: &str = r#"{
 "number":12,"title":"Header jumps when the menu opens","state":"OPEN","author":{"login":"alice"},
 "labels":[{"name":"bug"},{"name":"ui"}],
 "url":"https://github.com/acme/site/issues/12",
 "createdAt":"2026-10-10T09:00:00Z","updatedAt":"2026-10-16T12:00:00Z",
 "body":"<!-- please describe the problem -->\n## What happens\n\nOpening the **menu** shifts the header by `4px`.\n\n- Chrome 120\n- Firefox 121\n- [ ] not on Safari\n\n```css\nheader { position: sticky; }\n```\n> seen in production\n\nSee [the report](https://example.com/r/1).",
 "comments":[
  {"id":"c1","author":{"login":"bob"},"createdAt":"2026-10-11T09:00:00Z","body":"I can reproduce this on main."},
  {"id":"c2","author":{"login":"carol"},"createdAt":"2026-10-16T12:00:00Z","body":"Probably the scrollbar gutter.\nTrying a fix."}
 ]}"#;

pub const PR_31: &str = r#"{
 "number":31,"title":"Fix the sticky header","state":"OPEN","isDraft":false,"author":{"login":"bob"},
 "labels":[{"name":"ui"}],"url":"https://github.com/acme/site/pull/31",
 "createdAt":"2026-10-15T09:00:00Z","updatedAt":"2026-10-16T14:30:00Z",
 "baseRefName":"main","headRefName":"fix-header","reviewDecision":"CHANGES_REQUESTED",
 "additions":14,"deletions":3,
 "body":"Fixes #12 by reserving the scrollbar gutter.",
 "comments":[{"id":"c1","author":{"login":"alice"},"createdAt":"2026-10-16T14:30:00Z","body":"Thanks, looking."}],
 "reviews":[
  {"id":"r1","author":{"login":"alice"},"submittedAt":"2026-10-16T14:00:00Z","state":"CHANGES_REQUESTED","body":"The gutter should be a variable."},
  {"id":"r2","author":{"login":"carol"},"submittedAt":"2026-10-16T13:00:00Z","state":"COMMENTED","body":""}
 ],
 "statusCheckRollup":[
  {"__typename":"CheckRun","name":"lint","status":"COMPLETED","conclusion":"SUCCESS"},
  {"__typename":"CheckRun","name":"test (ubuntu)","status":"COMPLETED","conclusion":"FAILURE"},
  {"__typename":"CheckRun","name":"test (macos)","status":"IN_PROGRESS","conclusion":""},
  {"__typename":"CheckRun","name":"docs","status":"COMPLETED","conclusion":"SKIPPED"},
  {"__typename":"StatusContext","context":"ci/deploy","state":"SUCCESS"}
 ],
 "files":[
  {"path":"src/header.css","additions":10,"deletions":3,"changeType":"MODIFIED"},
  {"path":"src/layout.css","additions":4,"deletions":0,"changeType":"MODIFIED"}
 ]}"#;

pub const PR_31_COMMENTS: &str = r#"[
 {"user":{"login":"alice"},"body":"Use a variable here.","path":"src/header.css","line":12,"created_at":"2026-10-16T14:00:30Z","in_reply_to_id":null},
 {"user":{"login":"bob"},"body":"Done.","path":"src/header.css","line":12,"created_at":"2026-10-16T14:20:00Z","in_reply_to_id":991}
]"#;

pub const DIFF_31: &str = "diff --git a/src/header.css b/src/header.css\nindex 1..2 100644\n--- a/src/header.css\n+++ b/src/header.css\n@@ -1,3 +1,4 @@\n header {\n-  position: fixed;\n+  position: sticky;\n+  scrollbar-gutter: stable;\n }\n";
