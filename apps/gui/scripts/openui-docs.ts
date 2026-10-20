// OpenUI programs for looking at the window's drawing of each component (scene-openui.ts).

export interface SceneDoc {
  ask: string;
  before: string;
  program: string;
  after?: string;
  /** The fence is left open, as while the agent is still writing it. */
  streaming?: boolean;
}

const lines = (...parts: string[]): string => parts.join("\n");

/** Cards, stacks and tabs in turn, `levels` deep, with a field at the bottom. */
function deepProgram(levels: number): string {
  const wrap = (level: number, inner: string): string => {
    if (level % 3 === 0) return `Card([${inner}], "Level ${level}")`;
    if (level % 3 === 1) return `Stack([Text("Level ${level}", "muted"), ${inner}])`;
    return `Tabs([Tab("Level ${level}", [${inner}])])`;
  };
  const lines: string[] = [];
  for (let level = 0; level < levels - 1; level += 1) {
    lines.push(`${level === 0 ? "root" : `l${level}`} = ${wrap(level, `l${level + 1}`)}`);
  }
  lines.push(
    `l${levels - 1} = Stack([Text("The bottom."), FormControl("Deep field", Input("deep", "typed at the bottom"))])`,
  );
  return lines.join("\n");
}

export const DOCS: Record<string, SceneDoc> = {
  hello: {
    ask: "show me a card",
    before: "Here is a card with some text.",
    program: lines(
      'root = Card([intro, note], "Hello from OpenUI")',
      'intro = Text("A **card** with `markdown` inside it, and a [link](https://example.com).")',
      'note = Text("Muted text in a tone.", "muted")',
    ),
  },
  dashboard: {
    ask: "show me revenue",
    before: "Revenue over the last six months.",
    program: lines(
      'root = Card([header, kpis, tabs], "Revenue")',
      'header = Header("Monthly revenue", "Last 6 months")',
      'kpis = Stack([m1, m2, m3], "row")',
      'm1 = Metric("Revenue", "$25k", "+19%", "success")',
      'm2 = Metric("Customers", 412, "+12", "success")',
      'm3 = Metric("Churn", "2.1%", "+0.4pt", "danger")',
      'tabs = Tabs([Tab("Trend", [chart, spark]), Tab("Mix", [pie]), Tab("Accounts", [table])])',
      'chart = BarChart(["Jan", "Feb", "Mar", "Apr", "May", "Jun"], [Series("Revenue", [12, 15, 14, 18, 21, 25]), Series("Costs", [9, 9, 10, 11, 11, 12])])',
      'spark = Sparkline([12, 15, 14, 18, 21, 25], "Revenue")',
      'pie = PieChart([Slice("Pro", 62), Slice("Team", 28), Slice("Free", 10)], true)',
      'table = Table(["Account", "Plan", "MRR"], [["Acme", Tag("Pro", "accent"), 1200], ["Globex", Tag("Team"), 480], ["Initech", Tag("Free", "muted"), 0]])',
    ),
  },
  grid: {
    ask: "show me the disk",
    before: "Key numbers, then the details side by side.",
    program: lines(
      "root = Stack([kpis, details, more])",
      'kpis = Stack([used, free, largest], "row")',
      'used = Metric("Used", "820 GB", "of 896 GB", "danger")',
      'free = Metric("Free", "72 GB", "8%", "success")',
      'largest = Metric("Largest", "Documents", "355 GB")',
      "details = Grid([where, docs])",
      'where = Card([BarChart(["Documents", ".cache", "Games", "Other"], [Series("GB", [355, 65, 48, 103])], true)], "Where the space goes")',
      'docs = Card([Table(["Path", "Size"], [["neoworks", "210 GB"], ["clients", "45 GB"], ["other", "47 GB"]])], "Documents")',
      "more = Stack([c1, c2, c3])",
      'c1 = Card([Text("Three cards in a column become a grid.")], "One", "accent")',
      'c2 = Card([Text("Whatever the program says.")], "Two", "success")',
      'c3 = Card([Text("Tones colour the outline.")], "Three", "danger")',
    ),
  },
  layout: {
    ask: "show me layouts",
    before: "Rows, wrapping, justify and card variants.",
    program: lines(
      "root = Stack([row1, wrapped, between, variants, sep, header, steps])",
      'row1 = Stack([Tag("one", "accent"), Tag("two"), Tag("three", "success"), Button("A button", null, "secondary")], "row", "m", false, "center")',
      'wrapped = Stack([Tag("alpha"), Tag("beta"), Tag("gamma"), Tag("delta"), Tag("epsilon"), Tag("zeta"), Tag("eta"), Tag("theta"), Tag("iota"), Tag("kappa"), Tag("lambda"), Tag("mu"), Tag("nu"), Tag("xi"), Tag("omicron"), Tag("pi"), Tag("rho"), Tag("sigma"), Tag("tau"), Tag("upsilon"), Tag("phi")], "row", "s", true)',
      'between = Stack([Text("left"), Text("right", "muted")], "row", "m", false, "center", "between")',
      'variants = Stack([Card([Text("A card")], "card"), Card([Text("Sunk")], "sunk", null, "sunk"), Card([Text("Clear")], "clear", null, "clear")], "row")',
      "sep = Separator()",
      'header = Header("A header", "with a subtitle")',
      'steps = Steps([Step("Plan", "Decide what to do", "done"), Step("Build", "Write the code", "current"), Step("Ship", null, "pending"), Step("Rollback", "Only if needed", "failed")])',
    ),
  },
  content: {
    ask: "show me content",
    before: "Lists, key values, callouts, code and progress.",
    program: lines(
      "root = Stack([list, ordered, kv, callout, danger, code, progress, ents, indicators, link])",
      'list = List(["First item", "Second item", Tag("a tag", "accent")])',
      'ordered = List(["Plan", "Build", "Ship"], true)',
      'kv = KeyValue([KV("Host", "moritz-machine"), KV("Status", "healthy", "success"), KV("Errors", 3, "danger")])',
      'callout = Callout("Remember to **commit** before you switch branches.", "accent", "Heads up")',
      'danger = Callout("This deletes the build directory.", "danger")',
      'code = Code("fn main() {\\n    println!(\\"hello\\");\\n}", "rust")',
      'progress = Stack([Progress(0.72, "used"), Progress(0.4, "40% done")])',
      'ents = EntityList([ListItem("Ada Lovelace", "analyst", Tag("admin", "accent")), ListItem("Grace Hopper", "compilers and a long subtitle that goes on and on", "42"), ListItem("Linus", "kernel", 7)])',
      'indicators = Stack([MetricIndicator("1284", "12%", "up", null, "vs last week"), MetricIndicator("3.1s", "0.4s", "down", null, "p95 latency")], "row", "l")',
      'link = Link("Read the docs", "https://example.com/docs")',
    ),
  },
  charts: {
    ask: "show me charts",
    before: "A line, an area, a pie and horizontal bars.",
    program: lines(
      "root = Stack([line, area, row, bars])",
      'line = Card([LineChart(["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"], [Series("Visits", [120, 180, 150, 220, 260, 190, 240]), Series("Signups", [20, 35, 28, 50, 64, 40, 58])])], "Traffic")',
      'area = Card([AreaChart(["Jan", "Feb", "Mar", "Apr", "May", "Jun"], [Series("MRR", [10, 14, 13, 19, 22, 28])])], "Growth")',
      'row = Grid([Card([PieChart([Slice("Chrome", 64), Slice("Firefox", 18), Slice("Safari", 14), Slice("Other", 4)])], "Browsers"), Card([PieChart([Slice("Pro", 62), Slice("Team", 28), Slice("Free", 10)], true)], "Plans")])',
      'bars = Card([BarChart(["Documents", ".cache", "Games", "Other"], [Series("2025", [300, 60, 40, 90]), Series("2026", [355, 65, 48, 103])], true), BarChart(["Q1", "Q2", "Q3", "Q4"], [Series("A", [4, 6, 3, 8]), Series("B", [2, 3, 5, 4])], false, true)], "Bars")',
    ),
  },
  form: {
    ask: "I want to deploy",
    before: "Fill this in and I will deploy it.",
    program: lines(
      'root = Card([deploy], "Deploy")',
      'deploy = Form("deploy", [service, env, notes, replicas, dry, flags, when], buttons)',
      'service = FormControl("Service", Input("service", "api", "text", {required: true}), "The name in the registry")',
      'env = FormControl("Environment", Select("env", [Option("staging", "Staging"), Option("prod", "Production")], "Choose one", {required: true}))',
      'notes = FormControl("Notes", TextArea("notes", "Anything we should know"))',
      'replicas = FormControl("Replicas", Slider("replicas", 1, 10, 1))',
      'dry = FormControl("Options", CheckBox("dry", "Dry run"))',
      'flags = FormControl("Features", SwitchGroup("flags", [SwitchItem("beta", "Beta", "Enable the beta channel", true), SwitchItem("audit", "Audit log")]))',
      'when = FormControl("Date", DatePicker("when"))',
      'buttons = Buttons([Button("Run it", @ToAssistant("Deploy with these settings"), "primary"), Button("Cancel", null, "secondary"), Button("Docs", @OpenUrl("https://example.com"), "secondary")])',
    ),
  },
  choices: {
    ask: "pick a plan",
    before: "Choose a plan and the pieces you need.",
    program: lines(
      'root = Card([plan, radio, picks, toggles, follow], "Choices")',
      'plan = FormControl("Plan", OptionCards("plan", [OptionCard("free", "Free", "For trying it out"), OptionCard("pro", "Pro", "For teams that ship every day"), OptionCard("ent", "Enterprise", "Single sign-on and an audit trail")]))',
      'radio = FormControl("Region", RadioGroup("region", [Option("eu", "Europe"), Option("us", "United States"), Option("ap", "Asia Pacific")]))',
      'picks = FormControl("Extras", CheckBoxGroup("extras", [CheckBoxItem("ci", "CI minutes", "Faster builds", true), CheckBoxItem("sso", "SSO"), CheckBoxItem("audit", "Audit log", "Keep every change")]))',
      'toggles = FormControl("Notify", SwitchGroup("notify", [SwitchItem("mail", "Email", "Weekly digest", true), SwitchItem("chat", "Chat")]))',
      'follow = FollowUpBlock([FollowUpItem("Why is it slow?"), FollowUpItem("Show the queries"), FollowUpItem("Compare with last month")])',
    ),
  },
  disclosure: {
    ask: "show me tabs",
    before: "Tabs, an accordion and a carousel.",
    program: lines(
      "root = Stack([tabs, acc, car])",
      'tabs = Tabs([Tab("Overview", [Text("The **overview** tab.")]), Tab("Billing", [Text("Billing details."), Progress(0.3, "of quota")]), Tab("Team", [Text("Who is in it.")])])',
      'acc = Accordion([AccordionItem("What is it?", [Text("An accordion item that starts open.")], true), AccordionItem("How does it work?", [Text("Press the title or use Enter.")]), AccordionItem("Why?", [Text("Because.")])])',
      'car = Carousel([Card([Text("One")], "Slide 1"), Card([Text("Two")], "Slide 2"), Card([Text("Three")], "Slide 3"), Card([Text("Four")], "Slide 4"), Card([Text("Five")], "Slide 5")], 3)',
    ),
  },
  images: {
    ask: "show me pictures",
    before:
      "A web picture, then a gallery of a stored one, one on disk, a missing one and a web one.",
    program: lines(
      "root = Stack([one, gallery])",
      'one = Image("https://upload.wikimedia.org/wikipedia/commons/4/47/PNG_transparency_demonstration_1.png", "A web picture")',
      'gallery = ImageGallery([Image("{{ASSET}}", "From the asset store"), Image("{{FILE}}", "From disk"), Image("/nonexistent/second.png", "Missing"), Image("https://example.com/not-an-image", "Not an image")])',
    ),
  },
  places: {
    ask: "find me a hotel",
    before: "Three design hotels, each card a photo, a line on it and what to do next.",
    program: lines(
      "root = Grid([c1, c2, c3])",
      'c1 = Card([Image("https://www.superbude.com/site/assets/files/231380/superbude_hotel_hamburg_st__pauli_interior_dreibettbude_9.1600x0.jpg", "Superbude St. Pauli room"), Text("Playful design hostel in St. Pauli, a lively bar downstairs."), Text("**€270–340** for 3 nights", "muted"), b1], "Superbude St. Pauli")',
      'b1 = Buttons([Button("Website", @OpenUrl("https://www.superbude.com"), "secondary"), Button("More like this", @ToAssistant("More like Superbude"), "secondary")])',
      'c2 = Card([Image("https://image.motel-one.com/fileadmin/dam/Hotels/Hamburg/Hamburg_am_Michel/Redesign_2024_Rooms_Only/medium-MotelOne-HamburgamMichel_TheOne-3261.jpeg?w=1250&h=833&format=jpeg&quality=80&fit=cover", "Motel One room"), Text("Small, tidy, dark blue rooms by the Michel and the harbour."), Text("**€290–350** for 3 nights", "muted"), b2], "Motel One Michel")',
      'b2 = Buttons([Button("Website", @OpenUrl("https://www.motel-one.com"), "secondary"), Button("Compare", @ToAssistant("Compare Motel One"), "secondary")])',
      'c3 = Card([Image("{{FILE}}", "A picture on disk"), Text("A card whose picture is on disk, without buttons.")], "From disk")',
    ),
  },
  streaming: {
    ask: "ask me",
    before: "Fill this in.",
    streaming: true,
    program: lines(
      'root = Card([f, Text("Half way")], "Streaming")',
      'f = Form("s", [FormControl("Name", Input("name", "Your name")), FormControl("Pick", Select("pick", [Option("a", "A")]))], Buttons([Button("Send", null, "primary")]))',
      "half = Stack([Tex",
    ),
  },
  date: {
    ask: "when?",
    before: "Pick a day.",
    program: lines('root = Card([FormControl("Date", DatePicker("when"))], "When")'),
  },
  deep: {
    ask: "nest it",
    before: "Cards inside stacks inside tabs, a dozen levels down.",
    program: deepProgram(12),
  },
};
