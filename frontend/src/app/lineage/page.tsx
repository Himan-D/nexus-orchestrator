"use client";

import { useCallback, useEffect, useState } from "react";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { api, type LineageGraph, type LineageNode } from "@/lib/api";

const NODE_W = 190;
const NODE_H = 56;
const GAP_X = 40;
const GAP_Y = 90;

function relTime(t: string | null): string {
  if (!t) return "never";
  const mins = Math.floor((Date.now() - new Date(t).getTime()) / 60_000);
  if (mins < 1) return "just now";
  if (mins < 60) return `${mins}m ago`;
  const hours = Math.floor(mins / 60);
  if (hours < 24) return `${hours}h ago`;
  return `${Math.floor(hours / 24)}d ago`;
}

export default function LineagePage() {
  const [graph, setGraph] = useState<LineageGraph | null>(null);
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(async () => {
    try {
      setGraph(await api<LineageGraph>("/lineage"));
    } catch (e) {
      setError(e instanceof Error ? e.message : "Failed to load lineage");
    }
  }, []);

  useEffect(() => {
    load();
    const interval = setInterval(load, 5000);
    return () => clearInterval(interval);
  }, [load]);

  if (error) {
    return (
      <div className="mx-auto max-w-7xl p-8">
        <p className="text-sm text-destructive">{error}</p>
      </div>
    );
  }

  if (!graph) {
    return (
      <div className="mx-auto max-w-7xl p-8">
        <p className="text-muted-foreground text-sm">Loading lineage…</p>
      </div>
    );
  }

  // Group nodes by layer, then grid-place them.
  const layers = new Map<number, LineageNode[]>();
  for (const n of graph.nodes) {
    const bucket = layers.get(n.layer) ?? [];
    bucket.push(n);
    layers.set(n.layer, bucket);
  }
  const pos = new Map<string, { x: number; y: number }>();
  const maxInLayer = Math.max(...[...layers.values()].map((l) => l.length), 1);
  const width = layers.size * (NODE_W + GAP_X);
  const height = maxInLayer * (NODE_H + GAP_Y);
  for (const [layerIdx, nodes] of layers) {
    nodes.forEach((n, i) => {
      const x = layerIdx * (NODE_W + GAP_X);
      // Center each layer vertically.
      const layerH = nodes.length * (NODE_H + GAP_Y);
      const y = (height - layerH) / 2 + i * (NODE_H + GAP_Y);
      pos.set(n.key, { x, y });
    });
  }
  const center = (p: { x: number; y: number }, side: "l" | "r") => ({
    cx: side === "l" ? p.x : p.x + NODE_W,
    cy: p.y + NODE_H / 2,
  });

  const staleCount = graph.nodes.filter((n) => n.stale).length;

  return (
    <div className="mx-auto max-w-7xl p-8 flex flex-col gap-6">
      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-2xl font-bold tracking-tight">Lineage</h1>
          <p className="text-muted-foreground text-sm">
            Asset dependency graph derived from pipeline definitions. Left → right = upstream →
            downstream.
          </p>
        </div>
        <Badge variant="outline" className={staleCount > 0 ? "border-amber-500/30 bg-amber-500/10 text-amber-500" : ""}>
          {staleCount} of {graph.nodes.length} stale
        </Badge>
      </div>

      <Card>
        <CardHeader>
          <CardTitle>Asset graph</CardTitle>
        </CardHeader>
        <CardContent>
          {graph.nodes.length === 0 ? (
            <p className="text-muted-foreground text-sm py-8 text-center">
              No assets yet. Define pipelines with{" "}
              <code className="font-mono text-xs">materializes</code> and{" "}
              <code className="font-mono text-xs">asset_inputs</code> to build the graph.
            </p>
          ) : (
            <div className="overflow-x-auto">
              <svg
                width={width}
                height={height}
                viewBox={`0 0 ${width} ${height}`}
                className="min-w-full"
              >
                {graph.edges.map((e, i) => {
                  const from = pos.get(e.from);
                  const to = pos.get(e.to);
                  if (!from || !to) return null;
                  const a = center(from, "r");
                  const b = center(to, "l");
                  const mid = (a.cx + b.cx) / 2;
                  return (
                    <path
                      key={i}
                      d={`M ${a.cx} ${a.cy} C ${mid} ${a.cy}, ${mid} ${b.cy}, ${b.cx} ${b.cy}`}
                      fill="none"
                      stroke="currentColor"
                      strokeOpacity={0.25}
                      strokeWidth={1.5}
                      className="text-foreground"
                    />
                  );
                })}
                {graph.nodes.map((n) => {
                  const p = pos.get(n.key)!;
                  return (
                    <g key={n.key}>
                      <rect
                        x={p.x}
                        y={p.y}
                        width={NODE_W}
                        height={NODE_H}
                        rx={10}
                        className={
                          n.stale
                            ? "fill-amber-500/10 stroke-amber-500/50"
                            : "fill-emerald-500/5 stroke-emerald-500/40"
                        }
                        strokeWidth={1.5}
                      />
                      <text
                        x={p.x + 12}
                        y={p.y + 22}
                        className="fill-foreground font-mono"
                        fontSize={12}
                      >
                        {n.key.length > 24 ? n.key.slice(0, 23) + "…" : n.key}
                      </text>
                      <circle
                        cx={p.x + NODE_W - 14}
                        cy={p.y + 17}
                        r={4}
                        className={n.stale ? "fill-amber-500" : "fill-emerald-500"}
                      />
                      <text
                        x={p.x + 12}
                        y={p.y + 40}
                        className="fill-muted-foreground"
                        fontSize={10}
                      >
                        {n.stale ? "stale" : "fresh"} · {relTime(n.last_materialized_at)}
                      </text>
                    </g>
                  );
                })}
              </svg>
            </div>
          )}
        </CardContent>
      </Card>
    </div>
  );
}
