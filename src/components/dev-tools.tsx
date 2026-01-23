"use client";

import dynamic from "next/dynamic";

const Agentation = dynamic(
  () => import("agentation").then((mod) => mod.Agentation),
  { ssr: false }
);

export function DevTools() {
  // Check at runtime on the client, not at build time
  // This ensures dev tools work on production builds deployed to dev servers
  if (process.env.NODE_ENV !== "development") {
    return null;
  }
  return <Agentation />;
}
