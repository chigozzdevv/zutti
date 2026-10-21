import js from "@eslint/js";
import tseslint from "typescript-eslint";

export default [
  {
    ignores: [
      "target/**",
      "**/target/**",
      "node_modules/**",
      "**/node_modules/**",
      "**/dist/**",
      "**/generated/**",
    ],
  },
  { ...js.configs.recommended, files: ["tests/**/*.mjs"] },
  ...tseslint.configs.recommended.map((config) => ({
    ...config,
    files: ["packages/**/*.ts", "server/**/*.ts"],
  })),
  {
    files: ["packages/**/*.ts", "server/**/*.ts"],
    rules: {
      "@typescript-eslint/no-unused-vars": [
        "error",
        { argsIgnorePattern: "^_", varsIgnorePattern: "^_" },
      ],
    },
  },
  {
    files: ["tests/**/*.mjs"],
    languageOptions: {
      sourceType: "module",
      globals: { process: "readonly", console: "readonly", URL: "readonly", crypto: "readonly" },
    },
  },
];
