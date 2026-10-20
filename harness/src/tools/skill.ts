import type { AgentTool } from "../loop/index.ts";
import { Type } from "typebox";
import type { Skill } from "../skills.ts";

const parameters = Type.Object({
  name: Type.String({ description: "The skill's name, as the system prompt lists it." }),
});

export interface SkillDetails {
  name: string;
  lines: number;
}

/**
 * Returns a skill's text. It reads nothing but the skills the session found when it started, so
 * it never asks. The tool is there in every session, with or without skills.
 */
export function createSkillTool(skills: Skill[]): AgentTool<typeof parameters, SkillDetails> {
  return {
    name: "skill",
    label: "skill",
    description:
      "Load a skill: instructions for a kind of work, named in the system prompt. Returns its " +
      "text; load it before starting work it covers, and follow it.",
    parameters,
    execute(_toolCallId, { name }) {
      const wanted = name.trim().toLowerCase();
      const skill = skills.find((candidate) => candidate.name.toLowerCase() === wanted);
      if (!skill) {
        const known = skills.map((candidate) => candidate.name);
        const list = known.length > 0 ? `Skills: ${known.join(", ")}.` : "There are no skills.";
        return Promise.reject(new Error(`No skill named "${name}". ${list}`));
      }
      return Promise.resolve({
        content: [{ type: "text", text: skill.body }],
        details: { name: skill.name, lines: skill.body.split("\n").length },
      });
    },
  };
}
