import svg from '../../../assets/icon/icon-small.svg?raw';

export function GET() {
	return new Response(svg, { headers: { 'Content-Type': 'image/svg+xml' } });
}
