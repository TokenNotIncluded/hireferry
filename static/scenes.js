// HireFerry scenes, sampled with LIghtJUNction's particle-sculpture renderer.
// SPDX-License-Identifier: AGPL-3.0-or-later
import { Shape, rotate, tint, TAU } from './runtime/geometry.js';

const ink = [32, 83, 73], pale = [166, 193, 170], warm = [197, 147, 102];
function triangle(shape, a, b, c, color, part = 0, steps = 66) {
  for (let i = 0; i <= steps; i++) for (let j = 0; j <= steps - i; j++) {
    const u = i / steps, v = j / steps;
    shape.add([a[0] + (b[0]-a[0])*u + (c[0]-a[0])*v,
      a[1] + (b[1]-a[1])*u + (c[1]-a[1])*v,
      a[2] + (b[2]-a[2])*u + (c[2]-a[2])*v], color, part);
  }
}

export function ferry() {
  const s = new Shape();
  // A folded hull, with real front and rear surfaces and a central keel.
  const bow = [1.21,-.1,0], stern = [-1.21,-.1,0], keel = [0,-.73,0];
  triangle(s, stern, bow, [0,-.1,.48], tint(ink,.9));
  triangle(s, stern, bow, [0,-.1,-.48], pale);
  triangle(s, stern, [0,-.1,.48], keel, ink);
  triangle(s, [0,-.1,.48], bow, keel, tint(ink,1.15));
  triangle(s, bow, [0,-.1,-.48], keel, tint(ink,.85));
  triangle(s, [0,-.1,-.48], stern, keel, tint(ink,.7));
  triangle(s, [-.7,-.03,0], [.04,1.05,0], [.04,-.03,0], pale, 1, 60);
  triangle(s, [.11,.01,0], [.11,.85,0], [.8,.01,0], ink, 2, 58);
  s.line([.07,-.14,0],[.07,1.11,0],.013,warm,0,45);
  // Three separated wake arcs make the boat's direction legible.
  for (let i=0;i<3;i++) s.tube(t=>[-.95 + 1.9*t,-.91-i*.075,.2*Math.sin(t*Math.PI*2+i)], .006, pale,3,85,4);
  return { id: 'ferry', ...s.model(t => {
    const yaw = rotate(1,.35 + Math.sin(t*.25)*.2), roll = rotate(2,Math.sin(t*.8)*.035);
    return (p,v) => { if(p.part===3) v[1] += Math.sin(t*1.3+p.x*3)*.015; else { v[1]+=Math.sin(t*.8)*.04; roll(p,v); } yaw(p,v); };
  }, [-.15,.12]) };
}

export function bridge() {
  const s = new Shape();
  // Two banks are joined by an arch. Each profile has visible depth.
  s.box([-.87,-.43,0],[.25,.88,.42],ink);
  s.box([.87,-.43,0],[.25,.88,.42],ink);
  s.surface(90,24,(u,v)=>{
    const a=Math.PI*u;
    return [Math.cos(a)*.87,Math.sin(a)*.85-.03,(v-.5)*.43];
  },(u)=>tint(pale,.7+.3*Math.sin(Math.PI*u)));
  for(const z of [-.22,.22]) s.tube(t=>[Math.cos(Math.PI*t)*.87,Math.sin(Math.PI*t)*.85,z],.024,ink,0,90,8);
  for(let i=0;i<13;i++) {const x=-.78+i*.13, y=Math.sqrt(1-(x/.87)**2)*.85;
    s.line([x,y,.21],[x,y+.15,.21],.008,ink,0,8);
    s.line([x,y,-.21],[x,y+.15,-.21],.008,ink,0,8);
  }
  for(const z of [-.21,.21]) s.tube(t=>[Math.cos(Math.PI*t)*.87,Math.sin(Math.PI*t)*.85+.15,z],.012,ink,0,90,6);
  // Small figures on the banks represent the two sides.
  for(const [x,part,col] of [[-1.16,1,warm],[1.16,2,pale]]) {
    s.ellipsoid([x,-.12,0],[.11,.11,.11],col,part,450);
    s.ellipsoid([x,-.38,0],[.12,.19,.09],col,part,600);
  }
  return { id:'bridge', ...s.model(t=>{
    const yaw=rotate(1,Math.sin(t*.22)*.25);
    return (p,v)=>{if(p.part===1||p.part===2) v[1]+=Math.sin(t*1.2+p.part)*.025; yaw(p,v);};
  },[.2,.14]) };
}

export function compass() {
  const s = new Shape();
  s.ring([0,0,0],.92,.075,ink,0,2);
  s.ring([0,0,.02],.78,.012,pale,0,2);
  for(let i=0;i<48;i++) {const a=i/48*TAU,r=i%4===0 ? 0.63 : 0.69;
    s.line([Math.sin(a)*r,Math.cos(a)*r,.025],[Math.sin(a)*.75,Math.cos(a)*.75,.025],.009,i%4===0?ink:pale,0,5);
  }
  triangle(s,[0,.65,.06],[-.16,0,.06],[.16,0,.06],ink,1,58);
  triangle(s,[0,-.65,.06],[-.16,0,.06],[.16,0,.06],warm,1,58);
  triangle(s,[0,.65,-.06],[-.16,0,-.06],[.16,0,-.06],pale,1,48);
  triangle(s,[0,-.65,-.06],[-.16,0,-.06],[.16,0,-.06],warm,1,48);
  s.ellipsoid([0,0,.08],[.065,.065,.065],pale,0,800);
  return { id:'compass', ...s.model(t=>{
    const needle=rotate(2,Math.sin(t*.7)*.28-.36), yaw=rotate(1,Math.sin(t*.25)*.25);
    return (p,v)=>{if(p.part===1)needle(p,v);yaw(p,v);};
  },[.15,.12]) };
}

export const makeScenes = () => [ferry(), bridge(), compass()];
