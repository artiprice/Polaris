"""Validate the static Polaris prototype and optionally review it in Chromium.

No JavaScript is shipped in webui/. Browser assertions execute only in an
isolated test harness served from memory by a loopback-only HTTP server.
"""
from __future__ import annotations
import argparse
import contextlib
import hashlib
import html
from html.parser import HTMLParser
import http.server
import importlib.util
import json
from pathlib import Path
import re
import subprocess
import threading
import unittest
from urllib.parse import unquote, urlsplit
import xml.etree.ElementTree as ET

ROOT=Path(__file__).resolve().parent.parent
WEB=ROOT/'webui'
PAGES=['index','chat','history','models','settings','usage','states']


class PageParser(HTMLParser):
    def __init__(self):
        super().__init__(convert_charrefs=True)
        self.ids=[]; self.refs=[]; self.scripts=[]; self.controls=[]; self.labels=[]; self.elements=[]
    def handle_starttag(self,tag,attrs):
        a=dict(attrs);self.elements.append((tag,a))
        if 'id' in a:self.ids.append(a['id'])
        for key in ['href','src']:
            if key in a:self.refs.append((tag,key,a[key]))
        if tag=='script' or any(k.startswith('on') for k in a):self.scripts.append((tag,a))
        if tag in ['input','textarea','select']:self.controls.append((tag,a))
        if tag=='label' and 'for' in a:self.labels.append(a['for'])


def parse(path):
    p=PageParser();p.feed(path.read_text(encoding='utf-8'));return p


class WebuiChecks(unittest.TestCase):
    def test_all_pages_are_current_and_have_semantic_landmarks(self):
        spec=importlib.util.spec_from_file_location('webui_builder',ROOT/'scripts/build_webui.py')
        module=importlib.util.module_from_spec(spec);spec.loader.exec_module(module)
        for name,expected in module.render_pages().items():
            with self.subTest(page=name):
                text=(WEB/name).read_text(encoding='utf-8')
                self.assertEqual(text,expected,'Regenerate with scripts/build_webui.py')
                self.assertNotIn('\x00',text)
                p=parse(WEB/name)
                self.assertEqual(len(p.ids),len(set(p.ids)),'Duplicate IDs break label and anchor targets')
                self.assertEqual(sum(t=='main' for t,_ in p.elements),1)
                self.assertEqual(sum(t=='h1' for t,_ in p.elements),1)
                self.assertIn('Design preview',text)
                self.assertFalse(p.scripts,'Production pages must contain no runtime scripts or handlers')
    def test_local_links_fragments_assets_and_downloads_resolve(self):
        for name in PAGES:
            source=WEB/(name+'.html')
            for tag,key,ref in parse(source).refs:
                with self.subTest(page=name,reference=ref):
                    u=urlsplit(ref)
                    self.assertFalse(u.scheme or u.netloc,'Prototype runtime must be offline')
                    self.assertNotEqual(ref,'#','No fake links')
                    target=(source.parent/unquote(u.path)).resolve() if u.path else source
                    self.assertTrue(target.is_relative_to(WEB.resolve()))
                    self.assertTrue(target.is_file(),str(target))
                    if u.fragment and target.suffix=='.html':self.assertIn(u.fragment,parse(target).ids)
        for ref in re.findall(r'url\([\x27\x22]?([^\x27\x22)]+)',(WEB/'styles.css').read_text(encoding='utf-8')):
            self.assertTrue((WEB/ref).is_file(),ref)
        self.assertEqual(len((WEB/'downloads/pasted-content.txt').read_text(encoding='utf-8')),684)
        for file in (WEB/'downloads').iterdir():
            self.assertGreater(file.stat().st_size,80)
            self.assertIn('design sample',file.read_text(encoding='utf-8').lower())
    def test_controls_have_labels_and_unavailable_actions_do_not_pretend_to_execute(self):
        for page in PAGES:
            p=parse(WEB/(page+'.html'))
            for tag,a in p.controls:
                if a.get('type') not in ['checkbox','radio']:
                    self.assertTrue(a.get('aria-label') or a.get('id') in p.labels,(page,tag,a))
            for tag,a in p.elements:
                if tag=='button':self.assertIn('disabled',a,(page,a))
                self.assertNotIn('contenteditable',a)
    def test_fonts_and_brand_are_real_bundled_assets(self):
        for name,signature in [('inter.woff2',b'wOF2'),('space-grotesk.ttf',b'\x00\x01\x00\x00'),('jetbrains-mono.ttf',b'\x00\x01\x00\x00')]:
            data=(WEB/'assets/fonts'/name).read_bytes()
            self.assertGreater(len(data),10000)
            self.assertEqual(data[:4],signature)
        for name in ['Inter-OFL.md','Space-Grotesk-OFL.txt','JetBrains-Mono-OFL.txt']:
            self.assertIn('SIL OPEN FONT LICENSE',(WEB/'assets/fonts'/name).read_text(encoding='utf-8'))
        svg=ET.parse(WEB/'assets/polaris.svg')
        self.assertEqual(svg.getroot().attrib['viewBox'],'0 0 64 64')
    def test_usage_fixture_and_display_totals_are_consistent(self):
        spec=importlib.util.spec_from_file_location('webui_data',ROOT/'scripts/build_webui.py')
        module=importlib.util.module_from_spec(spec);spec.loader.exec_module(module)
        data=module.usage_data();self.assertEqual(len(data),364)
        self.assertEqual(len({r['day'] for r in data}),364)
        self.assertEqual(data[0]['day'].weekday(),0)
        self.assertTrue(all(r['sessions']>=0 and r['tokens']>=0 for r in data))
        page=(WEB/'usage.html').read_text(encoding='utf-8')
        self.assertIn('DEMO DATA',page)
        for value in [sum(r['sessions'] for r in data),sum(r['sessions']>0 for r in data)]:self.assertIn(str(value),page)
        self.assertEqual(page.count('class="day"'),364)

    def test_star_field_is_deterministic_tiny_and_irregular(self):
        spec=importlib.util.spec_from_file_location('webui_stars',ROOT/'scripts/build_webui.py')
        module=importlib.util.module_from_spec(spec);spec.loader.exec_module(module)
        rendered=module.render_stars()
        self.assertEqual(rendered,module.render_stars())
        coordinates=set()
        for name,svg in rendered.items():
            self.assertEqual((WEB/name).read_text(encoding='utf-8'),svg)
            root=ET.fromstring(svg);self.assertEqual(root.attrib['viewBox'],'0 0 1600 1000')
            stars=root.findall('{http://www.w3.org/2000/svg}rect')
            self.assertEqual(len(stars),800)
            for star in stars:
                a=star.attrib;x=int(a['x']);y=int(a['y'])
                self.assertEqual((a['width'],a['height']),('1','1'))
                self.assertTrue(0<=x<1600 and 0<=y<1000)
                self.assertNotIn((x,y),coordinates);coordinates.add((x,y))
        self.assertEqual(len(coordinates),2400)
        # New requirement: stars reach the prompt and the rectangular corners.
        self.assertGreater(sum(400<x<1200 and 250<y<600 for x,y in coordinates),300)
        for left,top in [(0,0),(1440,0),(0,900),(1440,900)]:
            self.assertGreaterEqual(sum(left<=x<left+160 and top<=y<top+100 for x,y in coordinates),6)
        cells=[sum(cx<=x<cx+200 and cy<=y<cy+200 for x,y in coordinates) for cx in range(0,1600,200) for cy in range(0,1000,200)]
        self.assertGreater(max(cells),min(cells)*2,'Clusters must not become a uniform grid')
        for page in PAGES:
            parser=parse(WEB/(page+'.html'))
            atmospheres=[a for _,a in parser.elements if a.get('class')=='welcome-atmosphere']
            self.assertEqual(len(atmospheres),int(page=='index'))
            for a in atmospheres:self.assertEqual(a.get('aria-hidden'),'true')

    def test_atmosphere_assets_are_local_deterministic_and_static(self):
        spec=importlib.util.spec_from_file_location('webui_atmosphere',ROOT/'scripts/build_webui.py')
        module=importlib.util.module_from_spec(spec);spec.loader.exec_module(module)
        assets=module.render_atmosphere_assets()
        self.assertEqual(assets,module.render_atmosphere_assets())
        for name,data in assets.items():
            self.assertEqual((WEB/name).read_bytes(),data.encode() if isinstance(data,str) else data)
        png=assets['assets/grain.png'];self.assertEqual(png[:8],b'\x89PNG\r\n\x1a\n')
        self.assertEqual(int.from_bytes(png[16:20],'big'),128)
        self.assertEqual(int.from_bytes(png[20:24],'big'),128)
        ribbon=ET.fromstring(assets['assets/aurora.svg'])
        self.assertTrue(ribbon.findall('.//{http://www.w3.org/2000/svg}path'))
        self.assertFalse(ribbon.findall('.//{http://www.w3.org/2000/svg}animate'))
        self.assertNotIn('<script',assets['assets/aurora.svg'])

    def test_minimal_composer_and_navigation_remain_complete(self):
        for page in PAGES:
            parser=parse(WEB/(page+'.html'))
            for _,a in parser.elements:
                if a.get('class')=='nav-link':self.assertTrue(a.get('aria-label'))
            self.assertFalse(any(a.get('class') in ['breadcrumbs','preview-badge','quick-starts','welcome-foot'] for _,a in parser.elements))
            if page in ['index','chat','states']:
                controls=[a for _,a in parser.controls if a.get('name','').endswith('-effort')]
                self.assertEqual([a['value'] for a in controls],['low','medium','high','xhigh','max'])
        states=parse(WEB/'states.html')
        groups=[a for tag,a in states.elements if tag=='details' and a.get('class')=='state-section']
        self.assertEqual(len(groups),7)
        self.assertTrue(all('open' not in a for a in groups))

    def test_compact_menus_have_scoped_native_groups_and_controls(self):
        spec=importlib.util.spec_from_file_location('webui_menu',ROOT/'scripts/build_webui.py')
        module=importlib.util.module_from_spec(spec);spec.loader.exec_module(module)
        first=PageParser();first.feed(module.composer('first'))
        second=PageParser();second.feed(module.composer('second'))
        self.assertTrue({a['name'] for _,a in first.controls if 'name' in a}.isdisjoint({a['name'] for _,a in second.controls if 'name' in a}))
        self.assertTrue(set(first.ids).isdisjoint(second.ids))
        for page in ['index','chat','states']:
            parser=parse(WEB/(page+'.html'))
            models=[a for tag,a in parser.controls if a.get('name','').endswith('-model')]
            modes=[a for tag,a in parser.controls if a.get('name','').endswith('-mode')]
            self.assertEqual([a['value'] for a in models],['super','nano9','nano8'])
            self.assertEqual([a['value'] for a in modes],['plan','auto'])
            self.assertEqual(len({a['name'] for a in models}),1)
            self.assertNotEqual(models[0]['name'],modes[0]['name'])
            for tag,a in parser.elements:
                if 'aria-controls' in a:self.assertIn(a['aria-controls'],parser.ids)
            self.assertEqual(sum(tag=='div' and a.get('class')=='popover-body' for tag,a in parser.elements),1)


    def test_chat_replacement_is_black_native_and_linked(self):
        page=(WEB/'chat.html').read_text(encoding='utf-8')
        parsed=parse(WEB/'chat.html')
        classes={value for _,a in parsed.elements for value in a.get('class','').split()}
        self.assertTrue({'chat-layout','chat-prompt','chat-response','chat-draft','chat-capsule','chat-file'}.issubset(classes))
        self.assertFalse({'chat-flow','user-message','assistant-message','artifact','welcome-atmosphere'} & classes)
        self.assertIn('Sample conversation',page)
        self.assertIn('No workspace files were read',page)
        self.assertIn('sending and uploads are not connected',page)
        downloads=[a for tag,a in parsed.elements if tag=='a' and 'download' in a]
        self.assertEqual(len(downloads),2)
        self.assertEqual({a['href'] for a in downloads},{'downloads/workspace-plan.md','downloads/validation.diff'})
        self.assertIn('latest',parsed.ids)
        self.assertIn('chat.html',(WEB/'history.html').read_text(encoding='utf-8'))
        # Chat specimens use the same native composer, file rows, and controls.
        states=(WEB/'states.html').read_text(encoding='utf-8')
        for component in ['chat-draft','chat-capsule','chat-file']:
            self.assertIn(component,states)
        welcome=(WEB/'index.html').read_text(encoding='utf-8')
        self.assertNotIn('chat-draft',welcome)
        self.assertNotIn('chat-capsule',welcome)


HARNESS=r'''<!doctype html><html><head><meta charset="utf-8"><style>html,body{margin:0;width:100%;height:100%}iframe{width:100%;height:100%;position:fixed;inset:0;border:0}#qa-results{display:none}</style></head><body><iframe id="subject" title="Polaris browser test"></iframe><pre id="qa-results"></pre><script>
const frame=document.getElementById('subject');
frame.onload=async()=>{
 const out={page:PAGE,width:innerWidth,height:innerHeight,checks:[],errors:[]};
 const test=(name,ok,detail='')=>{out.checks.push({name,ok:!!ok,detail});if(!ok)out.errors.push(name+': '+detail)};
 try{
  const d=frame.contentDocument,w=frame.contentWindow;
  const closedContent=e=>{for(let parent=e.parentElement;parent;parent=parent.parentElement){if(parent.tagName==='DETAILS'&&!parent.open&&!parent.querySelector(':scope>summary').contains(e))return true}return false};

  await d.fonts.ready;
  test('no page overflow',d.documentElement.scrollWidth<=w.innerWidth+1,JSON.stringify({width:w.innerWidth,scroll:d.documentElement.scrollWidth}));
  const spill=[...d.querySelectorAll('main *')].filter(e=>{let c=w.getComputedStyle(e),r=e.getBoundingClientRect();return !closedContent(e)&&c.display!=='none'&&r.width>0&&(r.right>w.innerWidth+2||r.left< -2)&&!e.closest('pre')&&c.position!=='absolute'&&c.position!=='fixed'}).slice(0,6).map(e=>({tag:e.tagName,cls:e.className,right:e.getBoundingClientRect().right}));
  test('content fits viewport',spill.length===0,JSON.stringify(spill));
  for(const font of ['Inter','Space Grotesk','JetBrains Mono']){await d.fonts.load('14px "'+font+'"');test('local font '+font,d.fonts.check('14px "'+font+'"'))}
  const context=d.querySelector('.context-toggle');test('context initially closed',!context.open);context.querySelector('summary').click();test('context opens',context.open);
  let r=context.querySelector('aside').getBoundingClientRect();test('open context fits',r.left>=-1&&r.right<=w.innerWidth+1,JSON.stringify({left:r.left,right:r.right}));context.querySelector('summary').click();test('context closes',!context.open);
  test('pure black canvas',w.getComputedStyle(d.body).backgroundColor==='rgb(0, 0, 0)');
  if(w.innerWidth>=768)test('compact icon rail',d.querySelector('.sidebar').getBoundingClientRect().width===68);
  for(const c of d.querySelectorAll('.composer,.composer textarea')){const style=w.getComputedStyle(c);test('opaque black composer '+c.tagName,style.backgroundColor==='rgb(0, 0, 0)'&&style.backgroundImage==='none'&&style.opacity==='1',style.backgroundColor+' / '+style.backgroundImage);if(c.classList.contains('composer'))test('composer has no backdrop blur',style.backdropFilter==='none')}
  if(PAGE==='chat'){
   test('chat has no welcome decoration',!d.querySelector('.welcome-atmosphere,.star-layer,.atmosphere-light'));
   test('chat uses the replacement composition',!!d.querySelector('.chat-layout')&&!d.querySelector('.chat-flow,.user-message,.assistant-message,.artifact'));
   const transcript=d.querySelector('.chat-transcript'),draft=d.querySelector('.chat-draft'),area=d.querySelector('textarea');
   test('reading column bounded',transcript.getBoundingClientRect().width<=761);
   for(const surface of d.querySelectorAll('.chat-prompt,.chat-file,.code-block,.code-head,.chat-dock,.activity .tool-body')){const s=w.getComputedStyle(surface);test('chat surface black '+surface.className,s.backgroundColor==='rgb(0, 0, 0)'&&s.backgroundImage==='none'&&s.backdropFilter==='none')}
   const inputBox=area.getBoundingClientRect(),toolbar=d.querySelector('.chat-draft-toolbar').getBoundingClientRect();
   test('textarea above actions',inputBox.bottom<=toolbar.top);
   test('new composer shape',w.getComputedStyle(draft).borderRadius==='22px');
   test('goal aligned right',w.getComputedStyle(d.querySelector('.goal-line')).justifyContent==='flex-end');
   for(const file of d.querySelectorAll('.chat-file a[download]')){const response=await fetch(file.href);test('real downloadable sample '+file.getAttribute('href'),response.ok&&(await response.text()).toLowerCase().includes('design sample'))}
   const filename=d.querySelector('.chat-file-copy strong'),original=filename.textContent;filename.textContent='a-very-long-review-filename-'.repeat(12)+'.md';test('long file name stays in viewport',d.documentElement.scrollWidth<=w.innerWidth+1);filename.textContent=original;
   const pre=d.querySelector('pre'),code=pre.querySelector('code'),originalCode=code.innerHTML;code.appendChild(d.createTextNode('long_code_identifier_'.repeat(100)));test('long code scrolls locally',pre.scrollWidth>pre.clientWidth&&d.documentElement.scrollWidth<=w.innerWidth+1);pre.scrollLeft=50;test('code horizontal scroll available',pre.scrollLeft>0);code.innerHTML=originalCode;pre.scrollLeft=0;
   test('line numbers excluded from copying',[...d.querySelectorAll('.line-number')].every(e=>w.getComputedStyle(e).userSelect==='none'));
   const latest=d.getElementById('latest');latest.scrollIntoView({block:'start',behavior:'instant'});const lastBox=latest.getBoundingClientRect(),dock=d.querySelector('.chat-dock').getBoundingClientRect();test('latest response clears sticky composer',lastBox.top>=0&&lastBox.bottom<=dock.top,JSON.stringify({latestTop:lastBox.top,latestBottom:lastBox.bottom,dockTop:dock.top}));
  }
  if(PAGE==='index'){
   const atmosphere=d.querySelector('.welcome-atmosphere'),bounds=atmosphere.getBoundingClientRect(),hero=d.querySelector('.welcome').getBoundingClientRect();
   test('atmosphere is decorative',atmosphere.getAttribute('aria-hidden')==='true'&&w.getComputedStyle(atmosphere).pointerEvents==='none');
   test('atmosphere spans welcome',bounds.width>=hero.width-1&&bounds.height>=hero.height-1);
   test('three slow twinkling layers',d.querySelectorAll('.star-layer').length===3&&[...d.querySelectorAll('.star-layer')].every(e=>parseFloat(w.getComputedStyle(e).animationDuration)>=11));
   const lights=[...d.querySelectorAll('.atmosphere-light')];test('four independently moving light layers',lights.length===4&&new Set(lights.map(e=>w.getComputedStyle(e).animationName)).size===4);
   const composer=d.querySelector('.composer'),box=composer.getBoundingClientRect();test('compact empty composer',box.height<=64,JSON.stringify({height:box.height}));
   const points=[];for(let i=1;i<=3;i++){const text=await(await fetch('/assets/stars-'+i+'.svg')).text();const svg=new DOMParser().parseFromString(text,'image/svg+xml');for(const star of svg.querySelectorAll('rect'))points.push([Number(star.getAttribute('x')),Number(star.getAttribute('y'))])}
   const originX=bounds.left+(bounds.width-1600)/2,originY=bounds.top+(bounds.height-1000)/2;let near=0;
   for(const [sx,sy] of points)for(let tx=-1;tx<=1;tx++)for(let ty=-1;ty<=1;ty++){const x=originX+sx+tx*1600,y=originY+sy+ty*1000;if(x<0||x>w.innerWidth||y<0||y>w.innerHeight)continue;const distance=Math.max(box.left-x,x-box.right,box.top-y,y-box.bottom,0);if(distance>=8&&distance<=24)near++}
   test('dense stars reach composer perimeter',near>=12,String(near));
   const animations=lights.flatMap(e=>e.getAnimations());for(const animation of animations){animation.pause();animation.currentTime=0;test('light animation uses transform and opacity',animation.effect.getKeyframes().every(k=>Object.keys(k).every(p=>['offset','computedOffset','easing','composite','transform','opacity'].includes(p))))}
   const first=lights.map(e=>w.getComputedStyle(e).transform);for(const animation of animations)animation.currentTime=12000;
   test('light transforms evolve over time',lights.every((e,i)=>w.getComputedStyle(e).transform!==first[i]));const after=composer.getBoundingClientRect();test('animation does not shift composer',after.top===box.top&&after.left===box.left&&after.width===box.width);
   for(const animation of animations)animation.play();
  }
  for(const info of d.querySelectorAll('.preview-info')){if(info.getBoundingClientRect().width===0)continue;info.querySelector('summary').click();const box=info.querySelector('.preview-description').getBoundingClientRect();test('preview disclosure visible and fits',info.open&&box.left>=0&&box.right<=w.innerWidth&&box.top>=0&&box.bottom<=w.innerHeight,JSON.stringify({left:box.left,right:box.right,top:box.top,bottom:box.bottom}));info.querySelector('summary').click()}
  for(const group of d.querySelectorAll('.state-section,.settings-disclosure')){test('secondary group initially closed',!group.open);group.querySelector('summary').click();test('secondary group opens',group.open)}
  for(const detail of d.querySelectorAll('.activity,.attachment-preview,.mobile-menu')){if(w.getComputedStyle(detail).display==='none')continue;const initial=detail.open;detail.querySelector('summary').click();test('native disclosure opens '+detail.className,detail.open!==initial);for(const control of detail.querySelectorAll('a,input')){control.focus();test('open disclosure controls accept focus',d.activeElement===control)}detail.open=initial}
  for(const detail of d.querySelectorAll('.composer-popover')){
   const summary=detail.querySelector('summary');summary.scrollIntoView({block:'center'});summary.click();const panel=detail.querySelector('.popover-body');
   test('menu opening animation present',w.getComputedStyle(panel).animationName==='menu-enter');
   const settle=()=>{panel.getBoundingClientRect();for(const animation of panel.getAnimations({subtree:true}))animation.finish()};settle();r=panel.getBoundingClientRect();
   test('composer menu fits',detail.open&&r.width>0&&r.left>=-1&&r.right<=w.innerWidth+1,JSON.stringify({left:r.left,right:r.right,width:r.width}));
   test('menu remains compact',r.width<=301&&r.height<=301,JSON.stringify({width:r.width,height:r.height}));
   const style=w.getComputedStyle(panel);test('menu is opaque black',style.backgroundColor==='rgb(0, 0, 0)'&&style.backgroundImage==='none'&&style.opacity==='1'&&style.backdropFilter==='none');
   const rows=[...panel.querySelectorAll('.menu-model-row')];
   for(const row of rows){row.click();settle();const input=row.querySelector('input'),visible=[...summary.querySelectorAll('.model-label')].filter(label=>w.getComputedStyle(label).display!=='none');test('local model selection reflects in trigger',input.checked&&visible.length===1&&visible[0].classList.contains('model-label-'+input.value));test('one model checkmark visible',rows.filter(label=>w.getComputedStyle(label.querySelector('.selection-check')).opacity==='1').length===1)}
   rows[0].click();
   for(const group of ['mode','effort'])for(const input of panel.querySelectorAll('input[name$="-'+group+'"]')){input.closest('label').click();settle();test('native '+group+' choice '+input.value,input.checked&&panel.querySelectorAll('input[name="'+input.name+'"]:checked').length===1)}
   panel.querySelector('input[value=plan]').closest('label').click();panel.querySelector('input[value=high]').closest('label').click();settle();
   for(const control of detail.querySelectorAll('a,input')){control.focus();test('open composer controls accept focus',d.activeElement===control)}
   summary.click();test('native summary closes menu',!detail.open);
  }
  for(const area of d.querySelectorAll('textarea')){const old=area.value;area.value='something';area.focus();area.setSelectionRange(4,4);area.setRangeText('text',4,4,'end');test('middle insertion keeps cursor',area.value==='sometextthing'&&area.selectionStart===8);area.setRangeText('\n',area.selectionStart,area.selectionEnd,'end');test('native multiline editing',area.value.includes('\n'));test('focused textarea stays black',w.getComputedStyle(area).backgroundColor==='rgb(0, 0, 0)');area.value='A long draft\n'.repeat(80);const height=area.getBoundingClientRect().height;const expanded=area.closest('.chat-draft'),limit=expanded?w.innerHeight*.375:180;test('multiline composer height bounded',height<=limit+1&&height>=24,JSON.stringify({height,limit}));if(expanded&&w.CSS.supports('field-sizing','content')){test('long draft uses expanded writing space',Math.abs(height-limit)<=1,JSON.stringify({height,limit}));let last=0;for(const lines of [2,6,12,24]){area.value=Array(lines).fill('Draft line').join('\n');const current=area.getBoundingClientRect().height,expected=Math.max(48,Math.min(limit,lines*24));test('draft grows naturally through '+lines+' lines',Math.abs(current-expected)<=1&&current>=last,JSON.stringify({current,expected}));last=current;}area.value='A long draft\n'.repeat(80);const originalFrameHeight=frame.style.height;for(const viewportHeight of [600,1200,1600]){frame.style.height=viewportHeight+'px';const actual=area.getBoundingClientRect().height;test('draft follows live viewport resize '+viewportHeight,Math.abs(actual-viewportHeight*.375)<=1,JSON.stringify({actual,expected:viewportHeight*.375}));}frame.style.height=originalFrameHeight;test('draft returns to original viewport size',Math.abs(area.getBoundingClientRect().height-limit)<=1);}test('multiline draft scrolls internally',area.scrollHeight>area.clientHeight);test('long draft stays black',w.getComputedStyle(area).backgroundColor==='rgb(0, 0, 0)');area.value=old;area.blur()}
  if(PAGE==='models'){const max=d.querySelector('input[value=max]');max.click();test('max effort selectable',max.checked&&w.getComputedStyle(max.closest('label')).color==='rgb(249, 226, 175)');const nano=d.querySelector('input[name=active-model][value="1"]');nano.click();test('model selection native',nano.checked&&d.querySelectorAll('input[name=active-model]:checked').length===1)}
  if(PAGE==='usage'){const models=d.getElementById('usage-models');models.click();test('models tab displays',w.getComputedStyle(d.querySelector('.models-view')).display==='block');test('overview tab hides',w.getComputedStyle(d.querySelector('.overview-view')).display==='none');d.getElementById('usage-overview').click();test('overview restores',w.getComputedStyle(d.querySelector('.overview-view')).display==='block')}
  if(PAGE==='settings'){const hi=d.getElementById('high-contrast'),before=w.getComputedStyle(d.body).getPropertyValue('--p-line');hi.click();test('contrast setting changes local tokens',w.getComputedStyle(d.body).getPropertyValue('--p-line')!==before);hi.click();const motion=d.getElementById('reduce-motion');motion.click();test('reduce motion local control',w.getComputedStyle(d.querySelector('.button')).transitionDuration==='0s');motion.click()}
  const focusable=[...d.querySelectorAll('a[href],summary,input:not(:disabled),textarea,select:not(:disabled)')].filter(e=>{const r=e.getBoundingClientRect();return !closedContent(e)&&r.width>0&&r.height>0&&w.getComputedStyle(e).visibility!=='hidden'});
  for(const e of focusable){e.focus();test('native focus '+(e.getAttribute('aria-label')||e.textContent||e.id).trim().slice(0,32),d.activeElement===e)}
  for(const e of [...d.querySelectorAll('details a, details input')].filter(closedContent)){e.focus();test('closed disclosure excludes focus',d.activeElement!==e)}
  for(const code of d.querySelectorAll('.code-block code'))test('code uses JetBrains Mono',w.getComputedStyle(code).fontFamily.includes('JetBrains Mono'));

  const ratio=(a,b)=>{const lum=c=>{let v=c.map(x=>{x/=255;return x<=.04045?x/12.92:Math.pow((x+.055)/1.055,2.4)});return .2126*v[0]+.7152*v[1]+.0722*v[2]};let x=lum(a),y=lum(b);return (Math.max(x,y)+.05)/(Math.min(x,y)+.05)};
  for(const [name,fg,bg]of [['body',[232,232,232],[0,0,0]],['muted',[150,156,148],[21,24,21]],['glass metadata',[160,170,150],[39,47,34]],['placeholder',[161,166,157],[29,32,27]],['primary button',[16,23,8],[118,185,0]],['error label',[229,162,162],[27,18,15]]])test('text contrast '+name,ratio(fg,bg)>=4.5,ratio(fg,bg).toFixed(2));
  if(PAGE==='chat')for(const selector of ['.chat-transcript','.chat-file-copy strong','.chat-file-copy small','.chat-eyebrow','.chat-draft-note','.chat-completion-time']){const color=w.getComputedStyle(d.querySelector(selector)).color.match(/[0-9]+/g).slice(0,3).map(Number);test('actual chat contrast '+selector,ratio(color,[0,0,0])>=4.5,ratio(color,[0,0,0]).toFixed(2))}
  if(PAGE==='states'){const target=d.getElementById('latest-message');d.documentElement.style.scrollBehavior='auto';target.scrollIntoView({behavior:'instant'});test('latest anchor reachable',target.getBoundingClientRect().top<w.innerHeight&&target.getBoundingClientRect().bottom>0)}
  const rootStyle=w.getComputedStyle(d.documentElement);test('shared palette loaded',rootStyle.getPropertyValue('--p-green').trim()==='#76b900');
 }catch(e){out.errors.push(e.stack||String(e))}
 document.getElementById('qa-results').textContent=JSON.stringify(out);
 document.documentElement.dataset.qaComplete='true';
};frame.src='/'+PAGE+'.html';
</script></body></html>'''



SCENES={
 'chat-long-draft-tall':('chat',"d.querySelector(\"textarea\").value=Array(80).fill(\"A line in a longer draft\").join(String.fromCharCode(10));d.querySelector(\".composer-wrap\").scrollIntoView({block:\"end\",behavior:\"instant\"});"),
 'chat-long-draft':('chat','d.querySelector("textarea").value=Array(30).fill("A line in a longer draft").join(String.fromCharCode(10));d.querySelector(".composer-wrap").scrollIntoView({block:"end",behavior:"instant"});'),
 'chat-long-draft-mobile':('chat','d.querySelector("textarea").value=Array(30).fill("A line in a longer draft").join(String.fromCharCode(10));d.querySelector(".composer-wrap").scrollIntoView({block:"end",behavior:"instant"});'),
 'chat-tools':('chat','d.querySelector(".activity").open=true;'),
 'chat-touch':('chat','d.getElementById("latest").scrollIntoView({block:"start",behavior:"instant"});d.querySelector(".composer-popover").open=true;'),
 'chat-reduced':('chat','d.querySelector(".activity").open=true;'),
 'chat-forced':('chat','d.querySelector(".activity").open=true;'),
 'chat-short':('chat','d.getElementById("latest").scrollIntoView({block:"start",behavior:"instant"});'),
 'chat-latest':('chat','d.documentElement.style.scrollBehavior="auto";d.getElementById("latest").scrollIntoView({block:"start",behavior:"instant"});'),
 'artifacts':('states','d.documentElement.style.scrollBehavior="auto";d.getElementById("attachments").scrollIntoView({block:"start",behavior:"instant"});'),
 'controls':('states','d.documentElement.style.scrollBehavior="auto";d.getElementById("controls").scrollIntoView({block:"start",behavior:"instant"});'),
 'composer':('states','d.documentElement.style.scrollBehavior="auto";d.getElementById("drafts").scrollIntoView({block:"start",behavior:"instant"});'),
 'model-effort':('models','d.documentElement.style.scrollBehavior="auto";d.getElementById("effort").scrollIntoView({block:"center",behavior:"instant"});'),
 'session-settings':('settings','d.documentElement.style.scrollBehavior="auto";d.getElementById("session").scrollIntoView({block:"start",behavior:"instant"});'),
 'usage-models':('usage','d.getElementById("usage-models").click();'),
 'chat-context':('chat','d.querySelector(".context-toggle").open=true;'),
 'menu-opening':('index','const menu=d.querySelector(".composer-popover");menu.open=true;for(const animation of menu.querySelector(".popover-body").getAnimations({subtree:true})){animation.pause();animation.currentTime=70;}'),
 'menu-selected':('index','const menu=d.querySelector(".composer-popover");menu.open=true;for(const value of ["nano9","auto","max"])menu.querySelector("input[value="+value+"]").closest("label").click();menu.getBoundingClientRect();for(const animation of menu.getAnimations({subtree:true}))animation.finish();'),
 'menu-reduced':('index','d.querySelector(".composer-popover").open=true;'),
 'menu-forced':('index','d.querySelector(".composer-popover").open=true;'),
 'menu-touch':('index','d.querySelector(".composer-popover").open=true;'),
 'menu-short':('index','d.querySelector("textarea").value=("Draft line"+String.fromCharCode(10)).repeat(20);d.querySelector(".composer-popover").open=true;'),
 'menu-chat-touch-short':('chat','d.querySelector("textarea").value=("Draft line"+String.fromCharCode(10)).repeat(20);d.querySelector(".composer-wrap").scrollIntoView({block:"end",behavior:"instant"});d.querySelector(".composer-popover").open=true;'),
 'menu-state-short':('states','d.querySelector("textarea").value=("Draft line"+String.fromCharCode(10)).repeat(20);d.querySelector(".composer-wrap").scrollIntoView({block:"end",behavior:"instant"});d.querySelector(".composer-popover").open=true;'),
 'menu-chat-short':('chat','d.querySelector("textarea").value=("Draft line"+String.fromCharCode(10)).repeat(20);d.querySelector(".composer-wrap").scrollIntoView({block:"end",behavior:"instant"});d.querySelector(".composer-popover").open=true;'),
 'menu-state':('states','d.querySelector("textarea").value=("Draft line"+String.fromCharCode(10)).repeat(20);d.querySelector(".composer-wrap").scrollIntoView({block:"end",behavior:"instant"});d.querySelector(".composer-popover").open=true;'),
 'welcome-model-menu':('index','const menu=d.querySelector(".composer-popover");menu.open=true;menu.querySelector("summary").scrollIntoView({block:"center"});'),
 'reduced-motion':('index',''),
 'forced-colors':('index',''),
 'atmosphere-0':('index','for(const animation of d.querySelector(".welcome-atmosphere").getAnimations({subtree:true})){animation.pause();animation.currentTime=0;}'),
 'atmosphere-12':('index','for(const animation of d.querySelector(".welcome-atmosphere").getAnimations({subtree:true})){animation.pause();animation.currentTime=12000;}'),
 'atmosphere-24':('index','for(const animation of d.querySelector(".welcome-atmosphere").getAnimations({subtree:true})){animation.pause();animation.currentTime=24000;}'),
 'state-fragment':('states','for(const group of d.querySelectorAll(".state-section"))group.open=false;d.defaultView.location.hash="attachments";'),
 'session-fragment':('settings','d.querySelector(".settings-disclosure").open=false;d.defaultView.location.hash="session";'),
 'welcome-long-draft':('index','d.querySelector("textarea").value="A multiline draft\\n".repeat(20);'),
 'solid-surface':('index','const style=d.createElement("style");style.textContent=".glass{backdrop-filter:none!important;background:#171c14!important}.composer,.composer textarea{backdrop-filter:none!important;background:#000!important}";d.head.append(style);'),
}


def scene_html(scene):
    page,action=SCENES[scene]
    template=r'''<!doctype html><html><head><meta charset="utf-8"><style>html,body{margin:0;height:100%}iframe{position:fixed;inset:0;width:100%;height:100%;border:0}#scene-results{display:none}</style></head><body><iframe id="subject" title="Polaris visual review"></iframe><pre id="scene-results"></pre><script>
const f=document.getElementById('subject');f.onload=async()=>{
 const d=f.contentDocument,w=f.contentWindow,scene=SCENE,out={scene:SCENE,checks:[],errors:[]};
 const test=(name,ok,detail='')=>{out.checks.push({name,ok:!!ok,detail});if(!ok)out.errors.push(name+': '+detail)};
 try{
  await d.fonts.ready;
  for(const group of d.querySelectorAll('.state-section,.settings-disclosure'))group.open=true;
  ACTION
  await new Promise(resolve=>setTimeout(resolve,100));
  test('scene fits viewport',d.documentElement.scrollWidth<=w.innerWidth+1);
  if(scene.startsWith('chat-')){
   const draft=d.querySelector('.chat-draft');test('chat composer present',!!draft);
   if(scene!=='chat-forced')test('chat remains full black',w.getComputedStyle(d.body).backgroundColor==='rgb(0, 0, 0)'&&w.getComputedStyle(draft).backgroundColor==='rgb(0, 0, 0)');
   if(scene==='chat-long-draft'||scene==='chat-long-draft-mobile'||scene==='chat-long-draft-tall'){
    const area=d.querySelector('textarea'),r=area.getBoundingClientRect(),limit=w.innerHeight*.375,toolbar=d.querySelector('.chat-draft-toolbar').getBoundingClientRect();
    test('expanded draft fills available writing space',Math.abs(r.height-limit)<=1,JSON.stringify({height:r.height,limit}));
    test('expanded draft and actions remain visible',r.top>=0&&toolbar.bottom<=w.innerHeight,JSON.stringify({top:r.top,toolbarBottom:toolbar.bottom}));
    test('overflow remains inside textarea',area.scrollHeight>area.clientHeight&&d.documentElement.scrollWidth<=w.innerWidth+1);
   }
   if(scene==='chat-reduced')test('chat motion removed',[...d.querySelectorAll('.chat-capsule,.activity-chevron,.composer-pill')].every(e=>w.getComputedStyle(e).transitionDuration==='0s'));
   if(scene==='chat-forced')test('chat system colors active',w.matchMedia('(forced-colors:active)').matches&&w.getComputedStyle(draft).borderTopStyle==='solid');
   if(scene==='chat-touch'){
    test('coarse chat pointer active',w.matchMedia('(pointer:coarse)').matches);
    const targets=[...d.querySelectorAll('.chat-capsule,.activity>summary,.chat-draft .composer-pill')].map(e=>e.getBoundingClientRect().height);test('chat touch targets 44px',targets.every(h=>h>=44),JSON.stringify(targets));
    const panel=d.querySelector('.popover-body');for(const animation of panel.getAnimations({subtree:true}))animation.finish();const r=panel.getBoundingClientRect();test('touch chat menu on screen',r.top>=0&&r.bottom<=w.innerHeight&&r.left>=0&&r.right<=w.innerWidth,JSON.stringify(r.toJSON()));
   }
   if(scene==='chat-latest'||scene==='chat-short'){const r=d.getElementById('latest').getBoundingClientRect(),dock=d.querySelector('.chat-dock').getBoundingClientRect();test('last response above composer',r.top>=0&&r.bottom<=dock.top,JSON.stringify({top:r.top,bottom:r.bottom,dockTop:dock.top}))}
  }
  if(scene==='reduced-motion'){
   test('reduced motion preference active',w.matchMedia('(prefers-reduced-motion: reduce)').matches);
   test('all atmosphere motion stops', [...d.querySelectorAll('.star-layer,.atmosphere-light')].every(e=>w.getComputedStyle(e).animationName==='none'));
  }
  if(scene==='forced-colors'){
   test('forced colors preference active',w.matchMedia('(forced-colors: active)').matches);
   test('decorative atmosphere removed',w.getComputedStyle(d.querySelector('.welcome-atmosphere')).display==='none');
  }
  if(scene==='state-fragment'||scene==='session-fragment'){
   const target=d.getElementById(scene==='state-fragment'?'attachments':'session');
   test('fragment reveals closed native disclosure',target.closest('details').open);
   const r=target.getBoundingClientRect();test('fragment target visible',r.top<w.innerHeight&&r.bottom>0,JSON.stringify({top:r.top,bottom:r.bottom}));
  }
  if(scene.startsWith('menu-')||scene==='welcome-model-menu'){
   const detail=d.querySelector('.composer-popover'),panel=detail.querySelector('.popover-body');
   if(scene!=='menu-opening')for(const animation of panel.getAnimations({subtree:true}))animation.finish();
   const r=panel.getBoundingClientRect();
   test('menu panel on screen',r.top>=0&&r.left>=0&&r.right<=w.innerWidth&&r.bottom<=w.innerHeight,JSON.stringify({top:r.top,bottom:r.bottom,left:r.left,right:r.right}));
   if(scene!=='menu-forced')test('menu surface pure black',w.getComputedStyle(panel).backgroundColor==='rgb(0, 0, 0)'&&w.getComputedStyle(panel).backdropFilter==='none');
   if(scene==='menu-opening')test('opening has an intermediate opacity',Number(w.getComputedStyle(panel).opacity)>0&&Number(w.getComputedStyle(panel).opacity)<1);
   if(scene==='menu-reduced')test('menu motion removed',[...detail.querySelectorAll('.popover-body,.menu-model-row')].every(e=>w.getComputedStyle(e).animationName==='none'));
   if(scene==='menu-selected'){const label=detail.querySelector('.model-label-nano9');test('selected trigger updates',w.getComputedStyle(label).display!=='none');test('mode and effort selections retained',detail.querySelector('input[value=auto]').checked&&detail.querySelector('input[value=max]').checked)}
   if(scene==='menu-touch'||scene==='menu-chat-touch-short'){test('coarse pointer emulation active',w.matchMedia('(pointer:coarse)').matches);const targets=[...detail.querySelectorAll('.menu-model-row,.mode-options label,.effort-options label')].map(e=>e.getBoundingClientRect().height);test('touch targets remain usable',targets.every(h=>h>=44),JSON.stringify(targets));}
   if(['menu-short','menu-touch','menu-chat-short','menu-chat-touch-short','menu-state-short'].includes(scene)){test('menu can scroll to footer',panel.scrollHeight>=panel.clientHeight);panel.scrollTop=panel.scrollHeight;const footer=panel.querySelector('footer').getBoundingClientRect();test('footer reachable',footer.top>=r.top&&footer.bottom<=r.bottom+1);panel.scrollTop=0}
  }
  if(scene==='welcome-model-menu'){
   const menu=d.querySelector('.popover-body'),r=menu.getBoundingClientRect();
   test('model menu fits vertically',r.top>=0&&r.bottom<=w.innerHeight,JSON.stringify({top:r.top,bottom:r.bottom}));
   test('model menu fits horizontally',r.left>=0&&r.right<=w.innerWidth);
  }
  if(scene==='solid-surface'||scene==='welcome-long-draft')test('fallback and long-draft composer black',w.getComputedStyle(d.querySelector('.composer')).backgroundColor==='rgb(0, 0, 0)'&&w.getComputedStyle(d.querySelector('textarea')).backgroundColor==='rgb(0, 0, 0)');
  if(scene.startsWith('atmosphere-')){test('light stays within welcome',d.querySelector('.welcome').scrollWidth<=w.innerWidth);test('opaque black prompt over light',w.getComputedStyle(d.querySelector('.composer')).backgroundColor==='rgb(0, 0, 0)')}
  if(scene==='welcome-long-draft'){
   const area=d.querySelector('textarea');test('long draft stays bounded',area.clientHeight<=180&&area.scrollHeight>area.clientHeight);
  }
 }catch(error){out.errors.push(error.stack||String(error))}
 document.getElementById('scene-results').textContent=JSON.stringify(out);document.documentElement.dataset.sceneReady='true';
};f.src=PAGE;</script></body></html>'''
    return template.replace('SCENE',json.dumps(scene)).replace('ACTION',action).replace('PAGE',json.dumps('/'+page+'.html'))


class ReviewServer(http.server.SimpleHTTPRequestHandler):
    def __init__(self,*args,**kwargs):super().__init__(*args,directory=str(WEB),**kwargs)
    def log_message(self,*args):pass
    def do_GET(self):
        route=urlsplit(self.path)
        if route.path=='/__scene__':
            if route.query not in SCENES:self.send_error(404);return
            body=scene_html(route.query).encode()
            self.send_response(200);self.send_header('Content-Type','text/html; charset=utf-8');self.send_header('Content-Length',str(len(body)));self.end_headers();self.wfile.write(body)
        elif route.path=='/__qa__':
            page=route.query
            if page not in PAGES:self.send_error(404);return
            body=HARNESS.replace('PAGE',json.dumps(page)).encode()
            self.send_response(200);self.send_header('Content-Type','text/html; charset=utf-8');self.send_header('Content-Length',str(len(body)));self.end_headers();self.wfile.write(body)
        else:super().do_GET()


def browser_review(binary: Path,output: Path,pages=None) -> dict:
    pages=pages or PAGES
    output=output.resolve()
    output.mkdir(parents=True,exist_ok=True)
    server=http.server.ThreadingHTTPServer(('127.0.0.1',0),ReviewServer)
    thread=threading.Thread(target=server.serve_forever,daemon=True);thread.start()
    base=f'http://127.0.0.1:{server.server_address[1]}'
    flags=[str(binary),'--headless','--in-process-gpu','--use-gl=angle','--use-angle=swiftshader','--enable-unsafe-swiftshader','--no-sandbox','--no-first-run','--hide-scrollbars',f'--log-file={output/"chromium.log"}','--timeout=12000','--virtual-time-budget=2500',f'--user-data-dir={output/"chromium-profile"}']
    results=[];screenshots=[];scenes=[]
    try:
        for width,height in [(1440,900),(1024,768),(390,844),(320,760)]:
            for page in pages:
                command=flags+[f'--window-size={width},{height}','--dump-dom',f'{base}/__qa__?{page}']
                r=subprocess.run(command,cwd=output,capture_output=True,text=True,encoding='utf-8',errors='replace',timeout=25)
                (output/f'{page}-{width}.qa.html').write_text(r.stdout,encoding='utf-8')
                match=re.search(r'<pre id="qa-results">(.*?)</pre>',r.stdout,re.S)
                if r.returncode or not match or not match.group(1):
                    raise RuntimeError(f'Browser harness failed for {page} at {width}: exit {r.returncode}; {r.stderr[-1000:]}')
                result=json.loads(html.unescape(match.group(1)));results.append(result)
                print(f'{page} {width}x{height}: {len(result["checks"])} checks, {len(result["errors"])} errors',flush=True)
                target=output/f'{page}-{width}.png'
                shot=subprocess.run(flags+[f'--window-size={width},{height}',f'--screenshot={target}',f'{base}/{page}.html'],cwd=output,capture_output=True,text=True,encoding='utf-8',errors='replace',timeout=25)
                if shot.returncode or not target.is_file():raise RuntimeError(f'Screenshot failed: {page} at {width}')
                screenshots.append(str(target.relative_to(ROOT)))
        for name,page,fragment,width,height in [
            ('chat-latest-desktop','chat','latest',1440,900),('chat-latest-mobile','chat','latest',390,844),
            ('artifacts-desktop','states','attachments',1440,900),('artifacts-mobile','states','attachments',390,844),
            ('controls-desktop','states','controls',1440,900),('composer-mobile','states','drafts',390,844),
        ]:
            if page not in pages:continue
            target=output/(name+'.png')
            r=subprocess.run(flags+[f'--window-size={width},{height}',f'--screenshot={target}',f'{base}/__scene__?'+{'latest':'chat-latest','attachments':'artifacts','controls':'controls','drafts':'composer'}[fragment]],cwd=output,capture_output=True,text=True,encoding='utf-8',errors='replace',timeout=25)
            if r.returncode or not target.exists():raise RuntimeError('Scene screenshot failed: '+name)
            screenshots.append(str(target.relative_to(ROOT)))
        for scene,width,height in [('chat-long-draft-tall',1440,1400),('chat-long-draft',1440,900),('chat-long-draft-mobile',390,844),('chat-tools',1440,900),('chat-touch',390,844),('chat-reduced',390,844),('chat-forced',390,844),('chat-short',1024,540),('model-effort',1440,900),('session-settings',390,844),('usage-models',1440,900),('chat-context',1440,900),('welcome-model-menu',390,844),('reduced-motion',1440,900),('solid-surface',1440,900),('forced-colors',1440,900),('welcome-long-draft',390,844),('state-fragment',390,844),('session-fragment',390,844),('atmosphere-0',1440,900),('atmosphere-12',1440,900),('atmosphere-24',1440,900),('menu-opening',1440,900),('menu-selected',1440,900),('menu-reduced',390,844),('menu-forced',390,844),('menu-touch',390,844),('menu-short',1024,540),('menu-chat-short',1024,540),('menu-chat-touch-short',390,540),('menu-state-short',390,540),('menu-state',390,844)]:
            if SCENES[scene][0] not in pages:continue
            target=output/(scene+'.png')
            extra=['--force-prefers-reduced-motion'] if scene in ['reduced-motion','menu-reduced','chat-reduced'] else ['--force-high-contrast'] if scene in ['forced-colors','menu-forced','chat-forced'] else ['--touch-events=enabled','--blink-settings=primaryPointerType=2,availablePointerTypes=2,primaryHoverType=0,availableHoverTypes=0'] if scene in ['menu-touch','chat-touch','menu-chat-touch-short'] else []
            r=subprocess.run(flags+extra+[f'--window-size={width},{height}','--dump-dom',f'--screenshot={target}',f'{base}/__scene__?{scene}'],cwd=output,capture_output=True,text=True,encoding='utf-8',errors='replace',timeout=25)
            if r.returncode or not target.exists():raise RuntimeError('Scene screenshot failed: '+scene)
            match=re.search(r'<pre id="scene-results">(.*?)</pre>',r.stdout,re.S)
            if not match or not match.group(1):raise RuntimeError('Scene assertions absent: '+scene)
            result=json.loads(html.unescape(match.group(1)));scenes.append(result)
            print(f'{scene}: {len(result["checks"])} checks, {len(result["errors"])} errors',flush=True)
            screenshots.append(str(target.relative_to(ROOT)))
        # Half the CSS viewport at 2x device scale approximates 200% page zoom.

        target=output/'welcome-2x-scale.png'
        scale=subprocess.run(flags+['--force-device-scale-factor=2','--window-size=720,450',f'--screenshot={target}',f'{base}/index.html'],cwd=output,capture_output=True,text=True,encoding='utf-8',errors='replace',timeout=25)
        screenshots.append(str(target.relative_to(ROOT))) if scale.returncode==0 and target.exists() else None
        report={'browser':str(binary),'results':results,'scenes':scenes,'screenshots':screenshots,'errors':[{'page':r['page'],'width':r['width'],'errors':r['errors']} for r in results if r['errors']]+[{'scene':s['scene'],'errors':s['errors']} for s in scenes if s['errors']],'limitations':['Focus checks call native focus APIs; physical keyboard and OS clipboard are not automated.','2x device scale is a layout approximation, not manual browser zoom.','CSS text contrast uses conservative declared surface bounds; screenshots also require visual review.']}
        (output/'browser-results.json').write_text(json.dumps(report,indent=2),encoding='utf-8')
        return report
    finally:
        server.shutdown();server.server_close();thread.join(timeout=5)


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--browser',type=Path)
    parser.add_argument('--pages',nargs='+',choices=PAGES,default=PAGES)
    parser.add_argument('--output',type=Path,default=ROOT/'target/webui-review')
    args=parser.parse_args()
    suite=unittest.defaultTestLoader.loadTestsFromTestCase(WebuiChecks)
    result=unittest.TextTestRunner(verbosity=2).run(suite)
    if not result.wasSuccessful():raise SystemExit(1)
    if args.browser:
        report=browser_review(args.browser,args.output,args.pages)
        print(json.dumps({'pages':len(args.pages),'viewport_cases':len(report['results']),'assertions':sum(len(r['checks']) for r in report['results'])+sum(len(s['checks']) for s in report['scenes']),'errors':report['errors'],'screenshots':len(report['screenshots'])},indent=2))
        if report['errors']:raise SystemExit(1)


if __name__=='__main__':main()
