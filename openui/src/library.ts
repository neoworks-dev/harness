// The suite@1 library: the components an OpenUI program may call, in the order docs/openui.md
// lists them. Positional arguments map by the key order of each schema, so that order is part of
// the format: stored programs carry no prop names, props are only ever added at the end.
import {
  createLibrary,
  defineAction,
  defineComponent,
  type DefinedComponent,
  markReactive,
  tagSchemaId,
  action as actionSchema,
} from "@openuidev/lang-core";
import { z } from "zod/v4";

export const LIBRARY_ID = "suite@1";

// Never rendered by lang-core: the daemon evaluates and the renderers draw.
const none = null;

/** Any component call; the prompt shows it as `Component`. */
const node = z.any();
tagSchemaId(node, "Component");

/** A table cell or list item: text, a number, a flag, or a component. */
const cell = z.any();
tagSchemaId(cell, "string | number | boolean | Component");

const tone = z.enum(["accent", "muted", "success", "danger"]);

/** `{required: true, minLength: 3}`: the spec's validation rules. */
const rules = z
  .object({
    required: z.boolean().optional(),
    email: z.boolean().optional(),
    url: z.boolean().optional(),
    numeric: z.boolean().optional(),
    min: z.number().optional(),
    max: z.number().optional(),
    minLength: z.number().optional(),
    maxLength: z.number().optional(),
    pattern: z.string().optional(),
  })
  .optional();

/** A prop a `$variable` may be bound to (two-way). */
function reactive<T extends z.ZodType>(schema: T): T {
  markReactive(schema);
  return schema;
}

function define<T extends z.ZodObject>(
  name: string,
  description: string,
  props: T,
): DefinedComponent<T, null> {
  return defineComponent({ name, props, description, component: none });
}

// ─── Layout ─────────────────────────────────────────────────────────────────

const Stack = define(
  "Stack",
  'Lays children out top to bottom, or side by side with direction "row"',
  z.object({
    children: z.array(node),
    direction: z.enum(["column", "row"]).optional(),
    gap: z.enum(["none", "s", "m", "l"]).optional(),
    wrap: z.boolean().optional(),
    align: z.enum(["start", "center", "end", "stretch"]).optional(),
    justify: z.enum(["start", "center", "end", "between"]).optional(),
  }),
);

const Grid = define(
  "Grid",
  "Lays children out in columns, left to right then down; columns is a maximum, a narrow screen uses fewer (default: as many as fit)",
  z.object({
    children: z.array(node),
    columns: z.number().optional(),
  }),
);

const Carousel = define(
  "Carousel",
  "Children side by side, as many as fit, with the rest reached by scrolling; visible is a maximum. Use it for more cards or items than a screen holds",
  z.object({ children: z.array(node), visible: z.number().optional() }),
);

const Card = define(
  "Card",
  "A framed group with an optional title; tone colors the frame, variant sunk fills it, clear has no frame",
  z.object({
    children: z.array(node),
    title: z.string().optional(),
    tone: tone.optional(),
    variant: z.enum(["card", "sunk", "clear"]).optional(),
  }),
);

const Header = define(
  "Header",
  "A heading with an optional subtitle",
  z.object({ title: z.string(), subtitle: z.string().optional() }),
);

const Separator = define("Separator", "A horizontal rule", z.object({}));

const Tab = define(
  "Tab",
  "One tab of Tabs",
  z.object({ label: z.string(), children: z.array(node) }),
);
const Tabs = define(
  "Tabs",
  "Switchable panels; only the selected tab is shown",
  z.object({ tabs: z.array(Tab.ref) }),
);

const AccordionItem = define(
  "AccordionItem",
  "One collapsible section of Accordion; open starts it expanded",
  z.object({ title: z.string(), children: z.array(node), open: z.boolean().optional() }),
);
const Accordion = define(
  "Accordion",
  "Collapsible sections",
  z.object({ items: z.array(AccordionItem.ref) }),
);

const Step = define(
  "Step",
  "One step of Steps",
  z.object({
    title: z.string(),
    detail: z.string().optional(),
    status: z.enum(["done", "current", "pending", "failed"]).optional(),
  }),
);
const Steps = define(
  "Steps",
  "A sequence of steps with their progress",
  z.object({ steps: z.array(Step.ref) }),
);

const Space = z.enum(["none", "s", "m", "l"]);

const Box = define(
  "Box",
  "A generic container: children in a column or a row (dir), with a gap, padding, a tone fill and a tone border. Sizes are shares: width is fill, fit or a share of the parent's width (0.3), never pixels or cells. Build what no specific component is: a swatch, a timeline, a bar of parts. Prefer Card, Stack and the specific components when one fits",
  z.object({
    children: z.array(node),
    dir: z.enum(["column", "row"]).optional(),
    gap: Space.optional(),
    pad: Space.optional(),
    align: z.enum(["start", "center", "end", "stretch"]).optional(),
    justify: z.enum(["start", "center", "end", "between"]).optional(),
    wrap: z.boolean().optional(),
    width: z.union([z.enum(["fill", "fit"]), z.number()]).optional(),
    fill: tone.optional(),
    border: tone.optional(),
    rounded: z.boolean().optional(),
  }),
);

// ─── Content ────────────────────────────────────────────────────────────────

const Text = define(
  "Text",
  "Markdown text; role title is a heading, code is monospace",
  z.object({
    text: z.string(),
    tone: tone.optional(),
    role: z.enum(["title", "body", "code"]).optional(),
  }),
);

const Code = define(
  "Code",
  "A block of code",
  z.object({ code: z.string(), language: z.string().optional() }),
);

const Table = define(
  "Table",
  "A table; cells are text, numbers or Tag, Link, Text",
  z.object({ columns: z.array(z.string()), rows: z.array(z.array(cell)) }),
);

const List = define(
  "List",
  "A bulleted or numbered list of text or components",
  z.object({ items: z.array(cell), ordered: z.boolean().optional() }),
);

const KV = define(
  "KV",
  "One pair of KeyValue",
  z.object({ key: z.string(), value: z.union([z.string(), z.number()]), tone: tone.optional() }),
);
const KeyValue = define(
  "KeyValue",
  "Aligned key and value lines",
  z.object({ pairs: z.array(KV.ref) }),
);

const Tag = define(
  "Tag",
  "A small label chip",
  z.object({ label: z.string(), tone: tone.optional() }),
);

const ListItem = define(
  "ListItem",
  "One row of EntityList; trailing is a value or a Tag",
  z.object({
    title: z.string(),
    subtitle: z.string().optional(),
    trailing: z.union([z.string(), z.number(), Tag.ref]).optional(),
  }),
);
const EntityList = define(
  "EntityList",
  "Rows of things with a title, a muted subtitle and a trailing value or Tag: people, files, services, tickets",
  z.object({ items: z.array(ListItem.ref) }),
);

const Callout = define(
  "Callout",
  "A highlighted note; text is markdown",
  z.object({ text: z.string(), tone: tone.optional(), title: z.string().optional() }),
);

const Metric = define(
  "Metric",
  "A headline number with an optional change",
  z.object({
    label: z.string(),
    value: z.union([z.string(), z.number()]),
    delta: z.string().optional(),
    tone: tone.optional(),
  }),
);

const MetricIndicator = define(
  "MetricIndicator",
  "A compact inline metric: a value with an optional change, an up or down arrow in the change's tone (default: up success, down danger) and a note; use it in cards, tables and lists where Metric is too big",
  z.object({
    value: z.union([z.string(), z.number()]),
    delta: z.string().optional(),
    direction: z.enum(["up", "down"]).optional(),
    tone: tone.optional(),
    subtext: z.string().optional(),
  }),
);

const Progress = define(
  "Progress",
  "A progress bar; value is 0 to 1",
  z.object({ value: z.number(), label: z.string().optional() }),
);

const Image = define(
  "Image",
  "An image: an asset handle, or the URL of the image file itself (png, jpeg, gif, webp), not of a page showing it",
  z.object({ src: z.string(), alt: z.string().optional() }),
);

const ImageGallery = define(
  "ImageGallery",
  "Several images of one subject: the first large, the others small beside it",
  z.object({ images: z.array(Image.ref) }),
);

const Link = define("Link", "A link", z.object({ text: z.string(), url: z.string() }));

const Shape = define(
  "Shape",
  "A shape drawn in a box of 0 to 1 on both sides, the left top corner 0,0, which the parent sizes to its width (aspect is width over height, default 2; wrap it in a Box to give it a share of the width). points are flat numbers, always 0 to 1: rect x,y,w,h; line x1,y1,x2,y2 and further points to connect; path the corners of a closed shape; arc cx,cy,r,from,to with r a share of the shorter side and from and to turns clockwise from the top (0.25 is 3 o'clock). tone colors it, fill fills it. Shapes next to each other share one box and draw over each other in order",
  z.object({
    kind: z.enum(["rect", "line", "path", "arc"]),
    points: z.array(z.number()),
    tone: tone.optional(),
    fill: z.boolean().optional(),
    aspect: z.number().optional(),
  }),
);

// ─── Charts ─────────────────────────────────────────────────────────────────

const Series = define(
  "Series",
  "One data series: a name and a value per label",
  z.object({ name: z.string(), values: z.array(z.number()) }),
);
const Slice = define(
  "Slice",
  "One slice of PieChart",
  z.object({ label: z.string(), value: z.number() }),
);

const BarChart = define(
  "BarChart",
  "Bars per label; one Series per bar color",
  z.object({
    labels: z.array(z.string()),
    series: z.array(Series.ref),
    horizontal: z.boolean().optional(),
    stacked: z.boolean().optional(),
  }),
);
const LineChart = define(
  "LineChart",
  "Lines over labels; one Series per line",
  z.object({ labels: z.array(z.string()), series: z.array(Series.ref) }),
);
const AreaChart = define(
  "AreaChart",
  "Filled areas over labels; one Series per area",
  z.object({ labels: z.array(z.string()), series: z.array(Series.ref) }),
);
const PieChart = define(
  "PieChart",
  "Shares of a whole; donut hollows it out",
  z.object({ slices: z.array(Slice.ref), donut: z.boolean().optional() }),
);
const Sparkline = define(
  "Sparkline",
  "A tiny inline trend",
  z.object({ values: z.array(z.number()), label: z.string().optional() }),
);

// ─── Forms ──────────────────────────────────────────────────────────────────

const Option = define(
  "Option",
  "A choice of Select or RadioGroup; label defaults to the value",
  z.object({ value: z.string(), label: z.string().optional() }),
);

const Input = define(
  "Input",
  "A single-line text field",
  z.object({
    name: z.string(),
    placeholder: z.string().optional(),
    type: z.enum(["text", "email", "url", "number", "password"]).optional(),
    rules,
    value: reactive(z.string().optional()),
  }),
);
const TextArea = define(
  "TextArea",
  "A multi-line text field",
  z.object({
    name: z.string(),
    placeholder: z.string().optional(),
    rules,
    value: reactive(z.string().optional()),
  }),
);
const Select = define(
  "Select",
  "A dropdown of options",
  z.object({
    name: z.string(),
    options: z.array(Option.ref),
    placeholder: z.string().optional(),
    rules,
    value: reactive(z.string().optional()),
  }),
);
const RadioGroup = define(
  "RadioGroup",
  "One choice out of a few options",
  z.object({
    name: z.string(),
    options: z.array(Option.ref),
    rules,
    value: reactive(z.string().optional()),
  }),
);
const OptionCard = define(
  "OptionCard",
  "A choice of OptionCards: a title and what it means",
  z.object({ value: z.string(), title: z.string(), description: z.string().optional() }),
);
const OptionCards = define(
  "OptionCards",
  "One choice out of a few options, each a card with a title and a description; use it over RadioGroup when the options need explaining",
  z.object({
    name: z.string(),
    options: z.array(OptionCard.ref),
    rules,
    value: reactive(z.string().optional()),
  }),
);
const CheckBox = define(
  "CheckBox",
  "A checkbox with a label",
  z.object({
    name: z.string(),
    label: z.string(),
    rules,
    value: reactive(z.boolean().optional()),
  }),
);
const Switch = define(
  "Switch",
  "An on/off switch with a label",
  z.object({ name: z.string(), label: z.string(), value: reactive(z.boolean().optional()) }),
);
const CheckBoxItem = define(
  "CheckBoxItem",
  "One checkbox of CheckBoxGroup; name is what the group's list holds when it is checked, checked starts it on",
  z.object({
    name: z.string(),
    label: z.string(),
    description: z.string().optional(),
    checked: z.boolean().optional(),
  }),
);
const CheckBoxGroup = define(
  "CheckBoxGroup",
  "Several checkboxes under one name, to pick any number of them; the value is the list of the checked names",
  z.object({
    name: z.string(),
    items: z.array(CheckBoxItem.ref),
    rules,
    value: reactive(z.array(z.string()).optional()),
  }),
);
const SwitchItem = define(
  "SwitchItem",
  "One switch of SwitchGroup; checked starts it on",
  z.object({
    name: z.string(),
    label: z.string(),
    description: z.string().optional(),
    checked: z.boolean().optional(),
  }),
);
const SwitchGroup = define(
  "SwitchGroup",
  "Several on/off switches under one name, for settings; the value is an object with each name on or off",
  z.object({
    name: z.string(),
    items: z.array(SwitchItem.ref),
    value: reactive(z.record(z.string(), z.boolean()).optional()),
  }),
);
const Slider = define(
  "Slider",
  "A number between min and max",
  z.object({
    name: z.string(),
    min: z.number(),
    max: z.number(),
    step: z.number().optional(),
    value: reactive(z.number().optional()),
  }),
);
const DatePicker = define(
  "DatePicker",
  "A date, YYYY-MM-DD",
  z.object({ name: z.string(), rules, value: reactive(z.string().optional()) }),
);

const FormControl = define(
  "FormControl",
  "A field of a form: a label over an input, with an optional hint",
  z.object({
    label: z.string(),
    input: z.union([
      Input.ref,
      TextArea.ref,
      Select.ref,
      RadioGroup.ref,
      OptionCards.ref,
      CheckBox.ref,
      Switch.ref,
      CheckBoxGroup.ref,
      SwitchGroup.ref,
      Slider.ref,
      DatePicker.ref,
    ]),
    hint: z.string().optional(),
  }),
);

const Button = define(
  "Button",
  "A button. Without an action it sends its label to the assistant",
  z.object({
    label: z.string(),
    action: actionSchema().optional(),
    variant: z.enum(["primary", "secondary", "danger"]).optional(),
  }),
);
const Buttons = define("Buttons", "A row of buttons", z.object({ buttons: z.array(Button.ref) }));

const FollowUpItem = define(
  "FollowUpItem",
  "One suggestion of FollowUpBlock",
  z.object({ text: z.string() }),
);
const FollowUpBlock = define(
  "FollowUpBlock",
  "Suggested next questions as pills at the end of a reply; choosing one sends its text to the assistant",
  z.object({ items: z.array(FollowUpItem.ref) }),
);

const Form = define(
  "Form",
  "Inputs submitted together; a button inside checks their rules first and a message carries their values",
  z.object({
    name: z.string(),
    children: z.array(node),
    buttons: z.optional(Buttons.ref),
  }),
);

// ─── Actions ────────────────────────────────────────────────────────────────

const Copy = defineAction({
  name: "Copy",
  description: "Copy text to the clipboard",
  params: z.object({ text: z.string() }),
});

export const library = createLibrary({
  id: LIBRARY_ID,
  root: "Stack",
  components: [
    Stack,
    Grid,
    Carousel,
    Card,
    Header,
    Separator,
    Tabs,
    Tab,
    Accordion,
    AccordionItem,
    Steps,
    Step,
    Box,
    Text,
    Code,
    Table,
    List,
    KeyValue,
    KV,
    Tag,
    EntityList,
    ListItem,
    Callout,
    Metric,
    MetricIndicator,
    Progress,
    Image,
    ImageGallery,
    Link,
    Shape,
    BarChart,
    LineChart,
    AreaChart,
    PieChart,
    Sparkline,
    Series,
    Slice,
    Form,
    FormControl,
    Input,
    TextArea,
    Select,
    Option,
    RadioGroup,
    OptionCards,
    OptionCard,
    CheckBox,
    Switch,
    CheckBoxGroup,
    CheckBoxItem,
    SwitchGroup,
    SwitchItem,
    Slider,
    DatePicker,
    Button,
    Buttons,
    FollowUpBlock,
    FollowUpItem,
  ],
  actions: [Copy],
  componentGroups: [
    {
      name: "Layout",
      components: [
        "Stack",
        "Grid",
        "Carousel",
        "Card",
        "Header",
        "Separator",
        "Tabs",
        "Tab",
        "Accordion",
        "AccordionItem",
        "Steps",
        "Step",
        "Box",
      ],
    },
    {
      name: "Content",
      components: [
        "Text",
        "Code",
        "Table",
        "List",
        "KeyValue",
        "KV",
        "Tag",
        "EntityList",
        "ListItem",
        "Callout",
        "Metric",
        "MetricIndicator",
        "Progress",
        "Image",
        "ImageGallery",
        "Link",
        "Shape",
      ],
    },
    {
      name: "Charts",
      components: [
        "BarChart",
        "LineChart",
        "AreaChart",
        "PieChart",
        "Sparkline",
        "Series",
        "Slice",
      ],
    },
    {
      name: "Forms",
      components: [
        "Form",
        "FormControl",
        "Input",
        "TextArea",
        "Select",
        "Option",
        "RadioGroup",
        "OptionCards",
        "OptionCard",
        "CheckBox",
        "Switch",
        "CheckBoxGroup",
        "CheckBoxItem",
        "SwitchGroup",
        "SwitchItem",
        "Slider",
        "DatePicker",
        "Button",
        "Buttons",
        "FollowUpBlock",
        "FollowUpItem",
      ],
    },
  ],
});

/** Components whose value the user edits: they get `key`, `value`, `error` and `required`. */
export const INPUTS: ReadonlySet<string> = new Set([
  "Input",
  "TextArea",
  "Select",
  "RadioGroup",
  "OptionCards",
  "CheckBox",
  "Switch",
  "CheckBoxGroup",
  "SwitchGroup",
  "Slider",
  "DatePicker",
]);
