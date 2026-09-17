import { invoke } from "@tauri-apps/api/core";
import { useEffect, useState } from "react";

type AppInfo = {
  name: string;
  version: string;
};

export function App() {
  const [info, setInfo] = useState<AppInfo | null>(null);

  useEffect(() => {
    void invoke<AppInfo>("app_info").then(setInfo);
  }, []);

  return (
    <main className="shell">
      <section className="hero">
        <p className="eyebrow">AngelicaProject</p>
        <h1>Aeria</h1>
        <p className="lede">
          Create, maintain, and review FINAL FANTASY XIV translations.
        </p>
        <div className="status">
          {info ? `${info.name} ${info.version}` : "Connecting to native core…"}
        </div>
      </section>
    </main>
  );
}
