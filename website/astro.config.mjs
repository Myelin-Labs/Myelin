import { defineConfig } from "astro/config";

export default defineConfig({
  site: "https://myelin.network",
  output: "static",
  devToolbar: {
    enabled: false,
  },
});
