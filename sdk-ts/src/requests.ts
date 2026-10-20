// What the client may ask the daemon, and what comes back. Each request is paired with the topic
// of its reply, and the payload types are read off the protocol's `Envelope`, so a call site gets
// the reply's payload with no checking of its own.
import type { Envelope } from "@suite/protocol/types";

type Topic = Envelope["topic"];

/** Requests that are answered with `bus.ack` only get the ack; the others get their own reply. */
export const REPLY_TOPICS = {
  "session.create": "session.info",
  "session.list": "session.list_result",
  "session.settings": "session.settings_result",
  "session.subscribe": "session.subscribed",
  "session.unsubscribe": "bus.ack",
  "session.cancel": "bus.ack",
  "session.input": "bus.ack",
  "app.presence": "bus.ack",
  "app.context": "bus.ack",
  "agent.move": "bus.ack",
  "connectors.list": "connectors.list_result",
  "view.subscribe": "view.snapshot",
  "view.unsubscribe": "bus.ack",
  "ui.action": "bus.ack",
  "asset.put": "asset.info",
  "asset.get": "asset.info",
  "asset.fetch": "asset.info",
} as const satisfies Partial<Record<Topic, Topic>>;

export type RequestTopic = keyof typeof REPLY_TOPICS;

type MessageOf<T extends Topic> = Extract<Envelope, { topic: T }>;

type PayloadOf<Message> = Message extends { payload: infer Payload } ? Payload : never;

/** The payload a request carries. */
export type RequestPayload<T extends RequestTopic> = PayloadOf<MessageOf<T>>;

/** What `request` resolves with: the reply's payload, or nothing for a plain ack. */
export type ReplyPayload<T extends RequestTopic> =
  PayloadOf<MessageOf<(typeof REPLY_TOPICS)[T]>> extends never
    ? void
    : PayloadOf<MessageOf<(typeof REPLY_TOPICS)[T]>>;

/** The arguments after the topic: the payload, unless the request has none. */
export type PayloadArgs<T extends RequestTopic> = [RequestPayload<T>] extends [never]
  ? []
  : [payload: RequestPayload<T>];
