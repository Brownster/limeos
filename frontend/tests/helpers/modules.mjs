// Load UI source files (TS and TSX) as ES modules in node, following their
// relative imports, so a component can be rendered with its real children.
// Each file becomes one data: URL (memoised, so a module shared by two
// importers is one instance). `stubs` maps a src-relative path ("api.ts") to
// replacement source for files that would reach the network or the DOM.
import { existsSync, readFileSync } from "node:fs";
import { createRequire } from "node:module";
import { dirname, join, normalize, relative } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import ts from "typescript";

const require = createRequire(import.meta.url);
const SRC = fileURLToPath(new URL("../../src/", import.meta.url));
const react = { "react/jsx-runtime": pathToFileURL(require.resolve("react/jsx-runtime")).href, react: pathToFileURL(require.resolve("react")).href };
const encode = source => `data:text/javascript;base64,${Buffer.from(source).toString("base64")}`;

function resolve(from, specifier) {
  const base = normalize(join(dirname(from), specifier));
  for (const candidate of [base, `${base}.ts`, `${base}.tsx`]) if (candidate.startsWith(SRC) && existsSync(candidate) && !candidate.endsWith("/")) return candidate;
  throw new Error(`Cannot resolve ${specifier} from ${from}`);
}

export function loader(stubs = {}) {
  const urls = new Map();
  const visiting = new Set();
  function url(file) {
    const key = relative(SRC, file);
    if (urls.has(key)) return urls.get(key);
    if (Object.hasOwn(stubs, key)) { const stub = encode(stubs[key]); urls.set(key, stub); return stub; }
    if (visiting.has(key)) throw new Error(`Import cycle through ${key}`);
    visiting.add(key);
    let output = ts.transpileModule(readFileSync(file, "utf8"), {
      fileName: file,
      compilerOptions: { module: ts.ModuleKind.ESNext, target: ts.ScriptTarget.ES2022, jsx: ts.JsxEmit.ReactJSX, verbatimModuleSyntax: false },
    }).outputText;
    output = output.replace(/(from\s+|import\s*\(\s*|import\s+)"([^"]+)"/g, (whole, lead, specifier) => {
      if (specifier in react) return `${lead}${JSON.stringify(react[specifier])}`;
      if (specifier.startsWith(".")) return `${lead}${JSON.stringify(url(resolve(file, specifier)))}`;
      return whole;
    });
    visiting.delete(key);
    const encoded = encode(output);
    urls.set(key, encoded);
    return encoded;
  }
  return {
    url: path => url(join(SRC, path)),
    load: path => import(url(join(SRC, path))),
  };
}
