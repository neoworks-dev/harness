// The favicon of each repository the sidebar shows, looked up on disk once per repository.
import { files } from "@neoworks/svelte-gpui";
import { dataUrl, FAVICON_PLACES, PROJECT_FOLDERS } from "./favicon.ts";

class Icons {
  /** A repository's favicon as a `data:` URL; null when it has none, absent until looked up. */
  byRepo = $state.raw<Readonly<Record<string, string | null>>>({});

  private asked = new Set<string>();

  /** Looks up the repositories not looked up yet. */
  ask(repos: readonly string[]): void {
    for (const repo of new Set(repos)) {
      if (this.asked.has(repo)) continue;
      this.asked.add(repo);
      void find(repo).then((url) => (this.byRepo = { ...this.byRepo, [repo]: url }));
    }
  }
}

/** The first favicon in the repository, then in its `apps/*` and `packages/*`, else null. */
async function find(repo: string): Promise<string | null> {
  for (const root of [repo, ...(await projects(repo))]) {
    for (const place of FAVICON_PLACES) {
      const path = `${root}/${place}`;
      const url = await read(path);
      if (url !== null) return url;
    }
  }
  return null;
}

/** The project directories of a monorepo, e.g. `apps/web`. */
async function projects(repo: string): Promise<string[]> {
  const found: string[] = [];
  for (const folder of PROJECT_FOLDERS) {
    try {
      const entries = await files.list(`${repo}/${folder}`);
      for (const entry of entries) if (entry.kind === "directory") found.push(entry.path);
    } catch {
      // No such folder.
    }
  }
  return found;
}

async function read(path: string): Promise<string | null> {
  try {
    if (!(await files.exists(path))) return null;
    return dataUrl(path, await files.readBytes(path));
  } catch {
    return null;
  }
}

export const icons = new Icons();
