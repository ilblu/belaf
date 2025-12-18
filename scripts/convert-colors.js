#!/usr/bin/env node

const multiplyMatrices = (A, B) => {
    return [
        A[0]*B[0] + A[1]*B[1] + A[2]*B[2],
        A[3]*B[0] + A[4]*B[1] + A[5]*B[2],
        A[6]*B[0] + A[7]*B[1] + A[8]*B[2]
    ];
}

const oklch2oklab = ([l, c, h]) => [
    l,
    isNaN(h) ? 0 : c * Math.cos(h * Math.PI / 180),
    isNaN(h) ? 0 : c * Math.sin(h * Math.PI / 180)
]

const rgb2srgbLinear = rgb => rgb.map(c =>
    Math.abs(c) <= 0.04045 ?
        c / 12.92 :
        (c < 0 ? -1 : 1) * (((Math.abs(c) + 0.055) / 1.055) ** 2.4)
)

const srgbLinear2rgb = rgb => rgb.map(c =>
    Math.abs(c) > 0.0031308 ?
        (c < 0 ? -1 : 1) * (1.055 * (Math.abs(c) ** (1 / 2.4)) - 0.055) :
        12.92 * c
)

const oklab2xyz = lab => {
    const LMSg = multiplyMatrices([
        1,  0.3963377773761749,  0.2158037573099136,
        1, -0.1055613458156586, -0.0638541728258133,
        1, -0.0894841775298119, -1.2914855480194092,
    ], lab)
    const LMS = LMSg.map(val => val ** 3)
    return multiplyMatrices([
         1.2268798758459243, -0.5578149944602171,  0.2813910456659647,
        -0.0405757452148008,  1.1122868032803170, -0.0717110580655164,
        -0.0763729366746601, -0.4214933324022432,  1.5869240198367816
    ], LMS)
}

const xyz2rgbLinear = xyz => {
    return multiplyMatrices([
        3.2409699419045226,  -1.537383177570094,   -0.4986107602930034,
       -0.9692436362808796,   1.8759675015077202,   0.04155505740717559,
        0.05563007969699366, -0.20397695888897652,  1.0569715142428786
    ], xyz)
}

const oklch2rgb = lch =>
    srgbLinear2rgb(xyz2rgbLinear(oklab2xyz(oklch2oklab(lch))))

const clamp = (val) => Math.max(0, Math.min(255, Math.round(val * 255)));

// Studio Brand Colors from globals.css
const colors = {
    primary: [0.8348, 0.1302, 160.9080],
    destructive: [0.5523, 0.1927, 32.7272],
    info: [0.6231, 0.1880, 259.8145],  // chart-2
    warning: [0.7686, 0.1647, 70.0804],  // chart-4
};

console.log('Converting OKLCH to RGB for CLI theme:\n');

for (const [name, oklch] of Object.entries(colors)) {
    const rgb = oklch2rgb(oklch);
    const r = clamp(rgb[0]);
    const g = clamp(rgb[1]);
    const b = clamp(rgb[2]);

    console.log(`${name}:`);
    console.log(`  OKLCH: oklch(${oklch.join(' ')})`);
    console.log(`  RGB:   Rgb(${r}, ${g}, ${b})`);
    console.log('');
}

console.log('\nRust code for theme.rs:\n');
for (const [name, oklch] of Object.entries(colors)) {
    const rgb = oklch2rgb(oklch);
    const r = clamp(rgb[0]);
    const g = clamp(rgb[1]);
    const b = clamp(rgb[2]);

    console.log(`pub fn ${name}() -> Rgb {`);
    console.log(`    Rgb(${r}, ${g}, ${b})`);
    console.log(`}`);
    console.log('');
}
