import js from "@eslint/js";

export default [
  { ignores: ["target/**", "node_modules/**", "**/generated/**"] },
  js.configs.recommended,
  { files: ["tests/**/*.mjs"], languageOptions: { sourceType: "module" } },
];
