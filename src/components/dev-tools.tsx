"use client";

import dynamic from "next/dynamic";

const Agentation = dynamic(
  () => import("agentation").then((mod) => mod.Agentation),
  { ssr: false }
);

function isRuntimeDev(): boolean {
  if (typeof window === "undefined") {
    return false;
  }

  const nextData = (window as Window & { __NEXT_DATA__?: { dev?: boolean } })
    .__NEXT_DATA__;
  if (typeof nextData?.dev === "boolean") {
    return nextData.dev;
  }

  const host = window.location.hostname;
  return host === "localhost" || host === "127.0.0.1";
}

export function DevTools() {
  const explicitlyEnabled = process.env.NEXT_PUBLIC_AGENTATION === "1";
  const isDev = process.env.NODE_ENV === "development" || isRuntimeDev();

  if (!explicitlyEnabled && !isDev) {
    return null;
  }
  return <Agentation />;
}
