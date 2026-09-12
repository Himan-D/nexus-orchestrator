"use client";

import { useEffect, useState } from "react";
import Link from "next/link";
import { useRouter } from "next/navigation";
import { Network } from "lucide-react";
import { Button } from "@/components/ui/button";
import { api, User } from "@/lib/api";

export function Nav() {
  const [user, setUser] = useState<User | null>(null);
  const [checked, setChecked] = useState(false);
  const router = useRouter();

  useEffect(() => {
    api<User>("/auth/me")
      .then(setUser)
      .catch(() => setUser(null))
      .finally(() => setChecked(true));
  }, []);

  const signout = async () => {
    await api("/auth/signout", { method: "POST" }).catch(() => null);
    router.push("/login");
    router.refresh();
  };

  return (
    <header className="border-b bg-background/95 backdrop-blur supports-[backdrop-filter]:bg-background/60">
      <div className="mx-auto flex h-14 max-w-7xl items-center justify-between px-6">
        <Link href="/" className="flex items-center gap-2 font-semibold tracking-tight">
          <div className="bg-primary/10 p-1.5 rounded-lg">
            <Network className="w-4 h-4 text-primary" />
          </div>
          Nexus
        </Link>
        <nav className="flex items-center gap-1 text-sm">
          <Link href="/jobs" className="px-3 py-1.5 rounded-md hover:bg-accent transition-colors">
            Pipelines
          </Link>
          <Link href="/assets" className="px-3 py-1.5 rounded-md hover:bg-accent transition-colors">
            Assets
          </Link>
          {checked && user && (
            <div className="flex items-center gap-3 ml-4 pl-4 border-l">
              <span className="text-muted-foreground text-xs">{user.email}</span>
              <Button variant="outline" size="sm" onClick={signout}>
                Sign out
              </Button>
            </div>
          )}
          {checked && !user && (
            <Link href="/login" className="px-3 py-1.5 rounded-md hover:bg-accent transition-colors ml-2">
              Sign in
            </Link>
          )}
        </nav>
      </div>
    </header>
  );
}
