"use client";

import { useCallback, useEffect, useState } from "react";
import Link from "next/link";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";
import { Badge } from "@/components/ui/badge";
import { api, type Asset } from "@/lib/api";

function relative(t: string | null): string {
  if (!t) return "never";
  const diff = Date.now() - new Date(t).getTime();
  const mins = Math.floor(diff / 60_000);
  if (mins < 1) return "just now";
  if (mins < 60) return `${mins}m ago`;
  const hours = Math.floor(mins / 60);
  if (hours < 24) return `${hours}h ago`;
  return `${Math.floor(hours / 24)}d ago`;
}

export default function AssetsPage() {
  const [assets, setAssets] = useState<Asset[]>([]);

  const load = useCallback(async () => {
    try {
      const data = await api<{ items: Asset[] }>("/assets?limit=200");
      setAssets(data.items);
    } catch {
      /* auth redirect handles 401 */
    }
  }, []);

  useEffect(() => {
    load();
    const interval = setInterval(load, 5000);
    return () => clearInterval(interval);
  }, [load]);

  return (
    <div className="mx-auto max-w-7xl p-8 flex flex-col gap-8">
      <div>
        <h1 className="text-2xl font-bold tracking-tight">Assets</h1>
        <p className="text-muted-foreground text-sm">
          The data your pipelines produce, with freshness at a glance.
        </p>
      </div>

      <Card>
        <CardHeader>
          <CardTitle>Catalog</CardTitle>
        </CardHeader>
        <CardContent>
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>Asset</TableHead>
                <TableHead>Freshness</TableHead>
                <TableHead>Last materialized by</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {assets.length === 0 && (
                <TableRow>
                  <TableCell colSpan={3} className="text-center h-24 text-muted-foreground">
                    No assets yet. Pipelines that declare{" "}
                    <code className="font-mono text-xs">materializes</code> populate this catalog.
                  </TableCell>
                </TableRow>
              )}
              {assets.map((a) => (
                <TableRow key={a.key}>
                  <TableCell>
                    <div className="font-mono text-sm">{a.key}</div>
                    {a.description && (
                      <p className="text-xs text-muted-foreground">{a.description}</p>
                    )}
                  </TableCell>
                  <TableCell>
                    <Badge
                      variant="outline"
                      className={
                        a.last_materialized_at
                          ? "border-emerald-500/30 bg-emerald-500/10 text-emerald-500"
                          : ""
                      }
                    >
                      {relative(a.last_materialized_at)}
                    </Badge>
                  </TableCell>
                  <TableCell>
                    {a.last_run_id ? (
                      <Link
                        href={`/runs/${a.last_run_id}`}
                        className="font-mono text-xs text-primary hover:underline"
                      >
                        run {a.last_run_id.substring(0, 8)}
                      </Link>
                    ) : (
                      <span className="text-muted-foreground text-xs">—</span>
                    )}
                  </TableCell>
                </TableRow>
              ))}
            </TableBody>
          </Table>
        </CardContent>
      </Card>
    </div>
  );
}
