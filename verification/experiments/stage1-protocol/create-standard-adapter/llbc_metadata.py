"""Bounded metadata selection from already compiler-validated LLBC JSON.

Unknown values are skipped with balanced delimiters and JSON string escapes.
Only small keys, options and three selected bodies reach stdlib json.loads.
No whole-file read, AST materialization, or proof evaluation occurs here.
"""
import json
import re

STRUCTURAL = re.compile(rb'[{}\[\]"]')
STRING_END = re.compile(rb'["\\]')
PRIMITIVE_END = re.compile(rb'[,}\]\s]')
MAX_SELECTED = 4*1024*1024


class Reader:
    def __init__(self, stream):
        self.stream = stream
        self.buffer = b''
        self.position = 0
        self.offset = stream.tell()

    def fill(self):
        if self.position < len(self.buffer):
            return True
        self.offset += len(self.buffer)
        self.buffer = self.stream.read(64*1024)
        self.position = 0
        return bool(self.buffer)

    def tell(self):
        return self.offset+self.position

    def peek(self):
        return self.buffer[self.position:self.position+1] if self.fill() else b''

    def whitespace(self):
        while self.fill():
            p=self.position
            while p < len(self.buffer) and self.buffer[p] in b' \t\r\n':
                p += 1
            self.position=p
            if p < len(self.buffer): return

    def expect(self, value):
        self.whitespace()
        assert self.peek()==value, ('expected JSON delimiter',value,self.tell())
        self.position += 1

    def value(self, keep=False, limit=MAX_SELECTED):
        self.whitespace()
        parts=[]
        length=0

        def consume(count):
            nonlocal length
            assert self.fill() and count <= len(self.buffer)-self.position
            if keep:
                length += count
                if length > limit: raise ValueError('selected metadata exceeds fixed memory bound')
                parts.append(self.buffer[self.position:self.position+count])
            self.position += count

        first=self.peek()
        if first not in (b'{',b'[',b'"'):
            assert first, 'unexpected JSON EOF'
            while self.fill():
                m=PRIMITIVE_END.search(self.buffer,self.position)
                end=m.start() if m else len(self.buffer)
                consume(end-self.position)
                if m: break
            return b''.join(parts) if keep else None
        consume(1)
        stack=[b'}' if first==b'{' else b']'] if first!=b'"' else []
        in_string=first==b'"'
        string_only=in_string
        while True:
            assert self.fill(), 'unexpected JSON EOF in skipped value'
            pattern=STRING_END if in_string else STRUCTURAL
            match=pattern.search(self.buffer,self.position)
            if match is None:
                consume(len(self.buffer)-self.position)
                continue
            consume(match.start()-self.position)
            token=self.peek()
            consume(1)
            if in_string:
                if token==b'\\':
                    consume(1)
                else:
                    in_string=False
                    if string_only: break
            elif token==b'"': in_string=True
            elif token in (b'{',b'['):
                stack.append(b'}' if token==b'{' else b']')
                if len(stack)>4096: raise ValueError('metadata skip depth bound exceeded')
            else:
                assert stack and stack.pop()==token, 'unbalanced skipped JSON'
                if not stack: break
        return b''.join(parts) if keep else None

    def small(self, limit=MAX_SELECTED):
        return json.loads(self.value(keep=True,limit=limit))

    def fields(self):
        self.expect(b'{')
        self.whitespace()
        if self.peek()==b'}':
            self.position += 1
            return
        while True:
            key=self.small(8192)
            assert isinstance(key,str)
            self.expect(b':')
            yield key
            self.whitespace()
            end=self.peek()
            assert end in (b',',b'}'), 'object delimiter missing'
            self.position += 1
            if end==b'}': return

    def items(self):
        self.expect(b'[')
        self.whitespace()
        if self.peek()==b']':
            self.position += 1
            return
        index=0
        while True:
            yield index
            index += 1
            self.whitespace()
            end=self.peek()
            assert end in (b',',b']'), 'array delimiter missing'
            self.position += 1
            if end==b']': return


def summary(path):
    selected=[]
    functions={}
    result={}
    with path.open('rb') as stream:
        reader=Reader(stream)

        def meta():
            name=[]
            opacity=None
            for key in reader.fields():
                if key=='name':
                    for _ in reader.items():
                        ident=None
                        for segment in reader.fields():
                            if segment=='Ident': ident=reader.small(8192)[0]
                            else: reader.value()
                        name.append(ident)
                elif key=='opacity': opacity=reader.small(8192)
                else: reader.value()
            return name,opacity

        def function():
            reader.whitespace()
            if reader.peek()==b'n':
                reader.value()
                return
            name,opacity,identity,body=[],None,None,None
            for key in reader.fields():
                if key=='item_meta': name,opacity=meta()
                elif key=='def_id': identity=reader.small(8192)
                elif key=='body':
                    reader.whitespace()
                    body=reader.tell()
                    reader.value()
                else: reader.value()
            brief=dict(id=identity,leaf=name[-1],prefix=name[:3],opacity=opacity)
            functions[identity]=brief
            if name[-1] and name[-1].startswith('proof_standard_create_'):
                assert body is not None
                # Independent handle does not disturb the bounded outer stream.
                with path.open('rb') as selected_stream:
                    selected_stream.seek(body)
                    value=Reader(selected_stream).small(MAX_SELECTED)
                calls=[]
                def visit(node):
                    if isinstance(node,dict):
                        if 'Call' in node:
                            called=node['Call']['call']['func'].get('Regular',{}).get('kind',{}).get('Fun')
                            if called is not None: calls.append(called)
                        for child in node.values(): visit(child)
                    elif isinstance(node,list):
                        for child in node: visit(child)
                visit(value)
                selected.append(dict(brief,arg_count=value['Structured']['locals']['arg_count'],calls=calls))

        for key in reader.fields():
            if key=='has_errors': result[key]=reader.small(8192)
            elif key=='translated':
                for translated in reader.fields():
                    if translated=='options': result['options']=reader.small()
                    elif translated=='target_information':
                        result['target_keys']=[item['key'] for item in reader.small()]
                    elif translated=='fun_decls':
                        for _ in reader.items(): function()
                    else: reader.value()
            else: reader.value()
    result['roots']=selected
    result['selected_callees']=[functions[i] for root in selected for i in root['calls']]
    result['reader_bounds']=dict(buffer_bytes=64*1024,
        selected_value_bytes=MAX_SELECTED,skip_depth=4096,whole_ast_loaded=False)
    return result
