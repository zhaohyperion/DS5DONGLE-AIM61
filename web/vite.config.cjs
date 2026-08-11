const SITE_CREATOR_PLACEHOLDER_DATABASE_ID = "00000000-0000-4000-8000-000000000000";

module.exports = async () => {
  process.env.WRANGLER_WRITE_LOGS ??= "false";
  process.env.WRANGLER_LOG_PATH ??= ".wrangler/logs";
  process.env.MINIFLARE_REGISTRY_PATH ??= ".wrangler/registry";

  const [{ default: vinext }, { cloudflare }, { sites }, { readFile }] = await Promise.all([
    import("vinext"),
    import("@cloudflare/vite-plugin"),
    import("./build/sites-vite-plugin.cjs"),
    import("node:fs/promises"),
  ]);
  const hostingConfig = JSON.parse(await readFile(`${__dirname}/.openai/hosting.json`, "utf8"));
  const { d1, r2 } = hostingConfig;

  return {
    plugins: [
      vinext(),
      sites(),
      cloudflare({
        viteEnvironment: { name: "rsc", childEnvironments: ["ssr"] },
        config: {
          main: "./worker/index.ts",
          compatibility_flags: ["nodejs_compat"],
          d1_databases: d1
            ? [{ binding: d1, database_name: "site-creator-d1", database_id: SITE_CREATOR_PLACEHOLDER_DATABASE_ID }]
            : [],
          r2_buckets: r2 ? [{ binding: r2, bucket_name: "site-creator-r2" }] : [],
        },
      }),
    ],
  };
};
