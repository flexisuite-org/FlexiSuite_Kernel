import type { NextConfig } from "next";
import path from "path";

const nextConfig: NextConfig = {
  // Point tracing to the monorepo root so build output paths resolve correctly
  outputFileTracingRoot: path.join(__dirname, "../../"),
};

export default nextConfig;
