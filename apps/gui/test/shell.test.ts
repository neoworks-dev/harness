import { expect, test } from "bun:test";
import { reflowCommand } from "../ui/lib/shell.ts";

test("lists break, pipelines stay whole while they fit", () => {
  expect(reflowCommand("cd ~/x && grep -rn a src | head -5; echo done", 30)).toBe(
    "cd ~/x &&\n  grep -rn a src | head -5;\n  echo done",
  );
});

test("a pipeline too long alone breaks after its pipes too", () => {
  expect(reflowCommand("cd ~/x && grep -rn 'a && b' src | head -5; echo done", 20)).toBe(
    "cd ~/x &&\n  grep -rn 'a && b' src |\n  head -5;\n  echo done",
  );
});

test("what fits, and what is inside $(…), quotes or a comment, stays", () => {
  expect(reflowCommand("a && b | c", 80)).toBe("a && b | c");
  const nested = "echo \"$(git log | head -1 && date)\" # x && y";
  expect(reflowCommand(nested, 10)).toBe(nested);
  expect(reflowCommand("make 2>&1 &> log & sleep 1", 5)).toBe("make 2>&1 &> log & sleep 1");
});

test("heredoc bodies are left alone", () => {
  const source = "cat > f.py <<'PY' && python3 f.py\nimport os; print(1) | 2; x && y\nPY\nls | wc -l";
  expect(reflowCommand(source, 12)).toBe(
    "cat > f.py <<'PY' &&\n  python3 f.py\nimport os; print(1) | 2; x && y\nPY\nls | wc -l",
  );
  expect(reflowCommand("x <<< a && y\nb && c", 5)).toBe("x <<< a &&\n  y\nb &&\n  c");
  expect(reflowCommand("echo $((1 << 2)) && y\nb", 5)).toBe("echo $((1 << 2)) &&\n  y\nb");
});

test("reflowed commands still parse", () => {
  for (const source of [
    "cd /tmp && ls -la | grep x || echo none; for i in 1 2; do echo $i; done",
    "if [ -f a ]; then cat a | sort; else echo b; fi && echo c",
    "cat <<EOF | wc -l && echo ok\nline one && two\nEOF\necho after; echo end",
    "case $x in a) echo a;; b) echo b;; esac && echo z",
    "git commit -m \"$(printf 'a\\nb')\" && git push 2>&1 | tail -3",
  ]) {
    const flowed = reflowCommand(source, 8);
    const check = Bun.spawnSync(["bash", "-n", "-c", flowed]);
    expect(check.exitCode, `${flowed}\n${check.stderr}`).toBe(0);
  }
});
