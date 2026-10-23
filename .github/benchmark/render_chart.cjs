#!/usr/bin/env node
'use strict';

// Render the public, validated single-node benchmark data to a GitHub README PNG.
// Chart.js produces a vertical, grouped bar chart. No fabricated measurements.
const fs = require('node:fs');
const path = require('node:path');
const { ChartJSNodeCanvas } = require('chartjs-node-canvas');
const { validatePublic, relativeScore } = require('./prepare_data.cjs');

const ORANGE = '#FF9136'; // JS: bright opaque orange
const BLUE = '#399DFF';   // Rust: bright opaque blue
const INK = '#24292F';
const MUTED = '#57606A';
const WIDTH = 1440;
const HEIGHT = 850;

function verify(value, why) {
  if (!value) throw new Error(`Invalid benchmark chart input: ${why}`);
}
function fmt(number) {
  return new Intl.NumberFormat('en-US', { maximumSignificantDigits: 4 }).format(number);
}
function writePng(png, targetPath) {
  verify(png.subarray(0, 8).equals(Buffer.from([137, 80, 78, 71, 13, 10, 26, 10])), 'renderer did not produce PNG');
  fs.mkdirSync(path.dirname(targetPath), { recursive: true });
  fs.writeFileSync(targetPath, png);
}

function pendingConfig(data) {
  return {
    type: 'bar',
    data: { labels: [], datasets: [] },
    options: { responsive: false, animation: false, devicePixelRatio: 1,
      plugins: { legend: { display: false } }, scales: { x: { display: false }, y: { display: false } } },
    plugins: [{ id: 'pendingRelease', afterDraw(chart) {
      const ctx = chart.ctx;
      ctx.save();
      ctx.textAlign = 'center';
      ctx.fillStyle = INK;
      ctx.font = 'bold 34px sans-serif';
      ctx.fillText(`Sub-Crate ${data.release_tag} · Benchmark pending`, WIDTH / 2, HEIGHT / 2 - 60);
      ctx.fillStyle = MUTED;
      ctx.font = '21px sans-serif';
      ctx.fillText('Waiting for verified measurements of the latest stable tag.', WIDTH / 2, HEIGHT / 2);
      ctx.fillText('Development commits and older releases are not shown.', WIDTH / 2, HEIGHT / 2 + 42);
      ctx.font = '16px sans-serif';
      ctx.fillText(`Release source: ${data.rust_revision}`, WIDTH / 2, HEIGHT / 2 + 100);
      ctx.restore();
    } }],
  };
}

function annotationsPlugin(data, scores) {
  return {
    id: 'originalMeasurements',
    afterDatasetsDraw(chart) {
      const ctx = chart.ctx;
      ctx.save();
      ctx.textBaseline = 'bottom';
      ctx.textAlign = 'center';
      ctx.fillStyle = INK;
      ctx.font = '600 14px sans-serif';
      chart.data.datasets.forEach((dataset, seriesIndex) => {
        const meta = chart.getDatasetMeta(seriesIndex);
        meta.data.forEach((element, i) => {
          const y = Math.max(chart.chartArea.top + 20, element.y - 8);
          ctx.fillText(fmt(seriesIndex === 0 ? 100 : scores[i]), element.x, y);
        });
      });
      ctx.restore();
    },
    afterDraw(chart) {
      const ctx = chart.ctx;
      const baseY = chart.chartArea.bottom + 75;
      ctx.save();
      ctx.textAlign = 'center';
      ctx.textBaseline = 'middle';
      data.metrics.forEach((metric, i) => {
        const x = chart.scales.x.getPixelForValue(i);
        ctx.font = '600 14px sans-serif';
        ctx.fillStyle = '#C56C19';
        ctx.fillText(`JS  ${fmt(metric.node)}`, x, baseY);
        ctx.fillStyle = '#1479CC';
        ctx.fillText(`Rust  ${fmt(metric.rust)}`, x, baseY + 24);
        ctx.font = '13px sans-serif';
        ctx.fillStyle = MUTED;
        ctx.fillText(metric.unit, x, baseY + 47);
        ctx.fillText(metric.better === 'higher' ? 'Raw: higher is better' : 'Raw: lower is better', x, baseY + 69);
      });
      ctx.strokeStyle = '#E1E4E8';
      ctx.lineWidth = 1;
      ctx.beginPath();
      ctx.moveTo(55, HEIGHT - 59);
      ctx.lineTo(WIDTH - 55, HEIGHT - 59);
      ctx.stroke();
      ctx.font = '12px sans-serif';
      ctx.textBaseline = 'middle';
      ctx.fillStyle = MUTED;
      ctx.textAlign = 'left';
      ctx.fillText('Score: JS = 100 · Higher is better for every metric · Raw values below.', 58, HEIGHT - 35);
      ctx.textAlign = 'right';
      const timestamp = `${data.generated_utc.slice(0, 16).replace('T', ' ')} UTC`;
      ctx.fillText(`${data.release_tag} · Run #${data.run_id}/${data.run_attempt} · ${data.rust_revision.slice(0, 12)} · ${timestamp}`, WIDTH - 58, HEIGHT - 35);
      ctx.restore();
    },
  };
}

async function main() {
  verify(process.argv.length === 4, 'usage: node render_chart.cjs data.json output.png');
  const dataPath = process.argv[2];
  const targetPath = process.argv[3];
  const data = JSON.parse(fs.readFileSync(dataPath, 'utf8'));
  validatePublic(data);

  const chart = new ChartJSNodeCanvas({
    width: WIDTH,
    height: HEIGHT,
    backgroundColour: '#FFFFFF',
  });
  if (data.status === 'pending') {
    writePng(await chart.renderToBuffer(pendingConfig(data), 'image/png'), targetPath);
    console.log(`Rendered waiting state for ${data.release_tag}`);
    return;
  }
  const scores = data.metrics.map(relativeScore);

  const config = {
    type: 'bar',
    data: {
      labels: [
        ['Pure', 'parsing'],
        ['Bulk', 'conversion'],
        ['JavaScript', 'pipeline'],
        ['8-client', 'throughput'],
        ['CPU', 'efficiency'],
        ['Settled', 'RSS'],
      ],
      datasets: [
        {
          label: 'Sub-Store · JS',
          data: data.metrics.map(() => 100),
          backgroundColor: ORANGE,
          borderColor: ORANGE,
          borderWidth: 0,
          borderRadius: 4,
          borderSkipped: false,
          categoryPercentage: 0.72,
          barPercentage: 0.84,
          maxBarThickness: 74,
        },
        {
          label: 'Sub-Crate · Rust',
          data: scores,
          backgroundColor: BLUE,
          borderColor: BLUE,
          borderWidth: 0,
          borderRadius: 4,
          borderSkipped: false,
          categoryPercentage: 0.72,
          barPercentage: 0.84,
          maxBarThickness: 74,
        },
      ],
    },
    plugins: [annotationsPlugin(data, scores)],
    options: {
      responsive: false,
      animation: false,
      devicePixelRatio: 1,
      layout: { padding: { top: 12, left: 20, right: 20, bottom: 166 } },
      plugins: {
        legend: {
          position: 'top',
          labels: { color: INK, font: { size: 16 }, boxWidth: 20, boxHeight: 15, padding: 21 },
        },
        title: {
          display: true,
          text: 'Sub-Store vs Sub-Crate · Performance',
          color: INK,
          font: { size: 28, weight: 'bold' },
          padding: { top: 11, bottom: 10 },
        },
        subtitle: {
          display: true,
          text: `Latest stable tag ${data.release_tag} · GitHub Actions · Linux x64 · Relative to JS baseline`,
          color: MUTED,
          font: { size: 15 },
          padding: { bottom: 16 },
        },
        tooltip: { enabled: false }, // GitHub README PNG has no interactive tooltip.
      },
      scales: {
        x: {
          stacked: false,
          grid: { display: false },
          ticks: { color: MUTED, font: { size: 16, weight: '500' }, padding: 12 },
          border: { color: '#D0D7DE' },
        },
        y: {
          beginAtZero: true,
          max: Math.ceil(Math.max(120, ...scores) * 1.19 / 20) * 20,
          title: { display: true, text: 'Performance score (JS = 100)', color: MUTED, font: { size: 15 } },
          grid: { color: '#EAECEF', lineWidth: 1 },
          ticks: {
            maxTicksLimit: 7,
            color: MUTED,
            font: { size: 14 },
            padding: 9,
            callback: value => fmt(value),
          },
          border: { display: false, dash: [3, 3] },
        },
      },
    },
  };

  const png = await chart.renderToBuffer(config, 'image/png');
  writePng(png, targetPath);
  console.log(`Rendered ${WIDTH}x${HEIGHT} Chart.js PNG for ${data.release_tag}, run #${data.run_id}/${data.run_attempt}`);
}

main().catch((error) => { console.error(error); process.exitCode = 1; });
