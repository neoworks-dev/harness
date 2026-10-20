import { expect, test } from "bun:test";
import { library } from "../src/library.ts";

// docs/openui.md, as lists. Stored programs carry no prop names, so this order is the format:
// props are only ever added at the end.
const ORDER: Record<string, string[]> = {
  Stack: ["children", "direction", "gap", "wrap", "align", "justify"],
  Card: ["children", "title", "tone", "variant"],
  Header: ["title", "subtitle"],
  Separator: [],
  Tabs: ["tabs"],
  Tab: ["label", "children"],
  Accordion: ["items"],
  AccordionItem: ["title", "children", "open"],
  Steps: ["steps"],
  Step: ["title", "detail", "status"],
  Text: ["text", "tone", "role"],
  Code: ["code", "language"],
  Table: ["columns", "rows"],
  List: ["items", "ordered"],
  KeyValue: ["pairs"],
  KV: ["key", "value", "tone"],
  Tag: ["label", "tone"],
  EntityList: ["items"],
  ListItem: ["title", "subtitle", "trailing"],
  Callout: ["text", "tone", "title"],
  Metric: ["label", "value", "delta", "tone"],
  Progress: ["value", "label"],
  Image: ["src", "alt"],
  Link: ["text", "url"],
  BarChart: ["labels", "series", "horizontal", "stacked"],
  LineChart: ["labels", "series"],
  AreaChart: ["labels", "series"],
  PieChart: ["slices", "donut"],
  Sparkline: ["values", "label"],
  Series: ["name", "values"],
  Slice: ["label", "value"],
  Form: ["name", "children", "buttons"],
  FormControl: ["label", "input", "hint"],
  Input: ["name", "placeholder", "type", "rules", "value"],
  TextArea: ["name", "placeholder", "rules", "value"],
  Select: ["name", "options", "placeholder", "rules", "value"],
  Option: ["value", "label"],
  RadioGroup: ["name", "options", "rules", "value"],
  OptionCards: ["name", "options", "rules", "value"],
  OptionCard: ["value", "title", "description"],
  CheckBox: ["name", "label", "rules", "value"],
  Switch: ["name", "label", "value"],
  Slider: ["name", "min", "max", "step", "value"],
  DatePicker: ["name", "rules", "value"],
  Button: ["label", "action", "variant"],
  Buttons: ["buttons"],
  FollowUpBlock: ["items"],
  FollowUpItem: ["text"],
};

test("every component takes its arguments in the order docs/openui.md lists", () => {
  const actual = Object.fromEntries(
    Object.entries(library.components).map(([name, def]) => [name, Object.keys(def.props.shape)]),
  );
  expect(actual).toEqual(ORDER);
});

test("the library has the Copy action", () => {
  expect(Object.keys(library.actions)).toEqual(["Copy"]);
});
