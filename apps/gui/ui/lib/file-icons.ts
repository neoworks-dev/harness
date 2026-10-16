// Icons for files by their name or extension, from the Material icon theme (material-icon-theme,
// MIT): the usual languages and formats, and a plain file for the rest. Coloured, so they are drawn
// as images, not tinted like `Icon`s.
import c from "material-icon-theme/icons/c.svg";
import console from "material-icon-theme/icons/console.svg";
import cpp from "material-icon-theme/icons/cpp.svg";
import css from "material-icon-theme/icons/css.svg";
import database from "material-icon-theme/icons/database.svg";
import docker from "material-icon-theme/icons/docker.svg";
import document from "material-icon-theme/icons/document.svg";
import file from "material-icon-theme/icons/file.svg";
import git from "material-icon-theme/icons/git.svg";
import go from "material-icon-theme/icons/go.svg";
import html from "material-icon-theme/icons/html.svg";
import image from "material-icon-theme/icons/image.svg";
import java from "material-icon-theme/icons/java.svg";
import javascript from "material-icon-theme/icons/javascript.svg";
import json from "material-icon-theme/icons/json.svg";
import kotlin from "material-icon-theme/icons/kotlin.svg";
import license from "material-icon-theme/icons/license.svg";
import lock from "material-icon-theme/icons/lock.svg";
import log from "material-icon-theme/icons/log.svg";
import lua from "material-icon-theme/icons/lua.svg";
import makefile from "material-icon-theme/icons/makefile.svg";
import markdown from "material-icon-theme/icons/markdown.svg";
import nix from "material-icon-theme/icons/nix.svg";
import pdf from "material-icon-theme/icons/pdf.svg";
import php from "material-icon-theme/icons/php.svg";
import python from "material-icon-theme/icons/python.svg";
import react from "material-icon-theme/icons/react.svg";
import react_ts from "material-icon-theme/icons/react_ts.svg";
import readme from "material-icon-theme/icons/readme.svg";
import ruby from "material-icon-theme/icons/ruby.svg";
import rust from "material-icon-theme/icons/rust.svg";
import settings from "material-icon-theme/icons/settings.svg";
import svelte from "material-icon-theme/icons/svelte.svg";
import svg from "material-icon-theme/icons/svg.svg";
import swift from "material-icon-theme/icons/swift.svg";
import toml from "material-icon-theme/icons/toml.svg";
import typescript from "material-icon-theme/icons/typescript.svg";
import vue from "material-icon-theme/icons/vue.svg";
import xml from "material-icon-theme/icons/xml.svg";
import yaml from "material-icon-theme/icons/yaml.svg";
import zip from "material-icon-theme/icons/zip.svg";
import { svgDataUrl } from "./base64.ts";

const BY_EXTENSION: Record<string, string> = {
  ts: typescript,
  mts: typescript,
  cts: typescript,
  tsx: react_ts,
  js: javascript,
  mjs: javascript,
  cjs: javascript,
  jsx: react,
  rs: rust,
  svelte: svelte,
  vue: vue,
  json: json,
  jsonc: json,
  json5: json,
  md: markdown,
  mdx: markdown,
  go: go,
  py: python,
  html: html,
  htm: html,
  css: css,
  scss: css,
  sass: css,
  less: css,
  yml: yaml,
  yaml: yaml,
  toml: toml,
  xml: xml,
  svg: svg,
  png: image,
  jpg: image,
  jpeg: image,
  gif: image,
  webp: image,
  ico: image,
  bmp: image,
  pdf: pdf,
  zip: zip,
  tar: zip,
  gz: zip,
  tgz: zip,
  xz: zip,
  c: c,
  h: c,
  cpp: cpp,
  cc: cpp,
  hpp: cpp,
  cxx: cpp,
  java: java,
  kt: kotlin,
  kts: kotlin,
  swift: swift,
  rb: ruby,
  php: php,
  lua: lua,
  nix: nix,
  sql: database,
  db: database,
  sqlite: database,
  sh: console,
  bash: console,
  zsh: console,
  fish: console,
  log: log,
  lock: lock,
  ini: settings,
  conf: settings,
  cfg: settings,
  env: settings,
  txt: document,
};

const BY_NAME: Record<string, string> = {
  Makefile: makefile,
  Dockerfile: docker,
  "README.md": readme,
  LICENSE: license,
  ".gitignore": git,
  ".gitattributes": git,
  "Cargo.lock": lock,
  "bun.lock": lock,
  "package-lock.json": lock,
};

const urls = new Map<string, string>();

/** The `data:` URL of the icon for a path, by its file name, else its extension. */
export function fileIcon(path: string): string {
  const name = path.slice(path.lastIndexOf("/") + 1);
  const dot = name.lastIndexOf(".");
  const extension = dot > 0 ? name.slice(dot + 1).toLowerCase() : "";
  const markup = BY_NAME[name] ?? BY_EXTENSION[extension] ?? file;
  let url = urls.get(markup);
  if (url === undefined) {
    url = svgDataUrl(markup);
    urls.set(markup, url);
  }
  return url;
}
