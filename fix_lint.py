#!/usr/bin/env python3
"""Fix markdown lint issues in .agents/skills/, CHANGELOG.md, docs/"""

import re
import sys
from pathlib import Path

def fix_file(path: Path) -> bool:
    content = path.read_text(encoding='utf-8')
    original = content
    
    # Fix MD041: Add h1 heading if missing (first non-frontmatter line should be h1)
    # Frontmatter ends at line with ---
    lines = content.split('\n')
    in_frontmatter = False
    frontmatter_end = 0
    for i, line in enumerate(lines):
        if line.strip() == '---':
            if not in_frontmatter:
                in_frontmatter = True
            else:
                frontmatter_end = i
                break
    
    # Check if there's an h1 after frontmatter
    has_h1 = False
    for line in lines[frontmatter_end+1:]:
        if line.strip().startswith('# '):
            has_h1 = True
            break
    
    if not has_h1 and 'name:' in content:
        # Extract name from frontmatter
        name_match = re.search(r'name:\s*(\S+)', content)
        if name_match:
            name = name_match.group(1).replace('-', ' ').title()
            # Insert h1 after frontmatter
            lines.insert(frontmatter_end + 1, f'# {name}')
            lines.insert(frontmatter_end + 2, '')
            content = '\n'.join(lines)
    
    # Fix MD040: Code blocks missing language - add 'text' to fenced blocks without language
    # Pattern: ```\n (start of code block without language)
    content = re.sub(r'^```\n', '```text\n', content, flags=re.MULTILINE)
    
    # Fix MD033: Inline HTML - replace common patterns
    content = content.replace('<what-to-do>', '')
    content = content.replace('</what-to-do>', '')
    content = content.replace('<supporting-info>', '')
    content = content.replace('</supporting-info>', '')
    content = content.replace('<refactor-plan-template>', '')
    content = content.replace('</refactor-plan-template>', '')
    content = content.replace('<spec-template>', '')
    content = content.replace('</spec-template>', '')
    content = content.replace('<user-story-example>', '')
    content = content.replace('</user-story-example>', '')
    content = content.replace('<questionnaire-template>', '')
    content = content.replace('</questionnaire-template>', '')
    content = content.replace('<question-example>', '')
    content = content.replace('</question-example>', '')
    content = content.replace('<vertical-slice-rules>', '')
    content = content.replace('</vertical-slice-rules>', '')
    content = content.replace('<local-ticket-template>', '')
    content = content.replace('</local-ticket-template>', '')
    content = content.replace('<issue-template>', '')
    content = content.replace('</issue-template>', '')
    content = content.replace('<example>', '')
    content = content.replace('</example>', '')
    
    # Fix MD049: Emphasis style - standardize on _ for emphasis
    # This is tricky - we need to be careful not to break bold/italic
    # For now, just fix the specific cases mentioned in CI
    
    # Fix MD028: Blank line inside blockquote
    content = re.sub(r'(> .+)\n>\n(> .+)', r'\1\n\2', content)
    
    # Fix MD032: List should be preceded/followed by blank line
    content = re.sub(r'([^\n])\n(- \[ \] )', r'\1\n\n\2', content)
    content = re.sub(r'(- \[ .+\])\n([^\n-])', r'\1\n\n\2', content)
    
    # Fix MD031: No blank line before fenced code block
    content = re.sub(r'([^\n])\n(```)', r'\1\n\n\2', content)
    
    # Fix MD022: Expected blank line above heading
    content = re.sub(r'([^\n])\n(## )', r'\1\n\n\2', content)
    
    # Fix MD076: Unexpected blank line between list items (CHANGELOG.md)
    # This is more complex - we'll handle CHANGELOG separately
    
    # Fix MD025: Multiple top-level headings - remove duplicate h1 if present
    h1_count = len(re.findall(r'^# ', content, flags=re.MULTILINE))
    if h1_count > 1:
        # Keep only the first one
        first = True
        new_lines = []
        for line in content.split('\n'):
            if line.startswith('# '):
                if first:
                    new_lines.append(line)
                    first = False
                # Skip subsequent h1s
            else:
                new_lines.append(line)
        content = '\n'.join(new_lines)
    
    if content != original:
        path.write_text(content, encoding='utf-8')
        return True
    return False

def fix_changelog(path: Path) -> bool:
    content = path.read_text(encoding='utf-8')
    original = content
    
    # MD076: Remove blank lines between list items
    # Pattern: list item, blank line, list item
    lines = content.split('\n')
    new_lines = []
    i = 0
    while i < len(lines):
        line = lines[i]
        new_lines.append(line)
        # Check if this is a list item and next non-empty is also a list item with blank line between
        if re.match(r'^\s*[-*+]\s', line):
            j = i + 1
            while j < len(lines) and lines[j].strip() == '':
                j += 1
            if j < len(lines) and re.match(r'^\s*[-*+]\s', lines[j]):
                # Skip the blank lines
                i = j - 1
        i += 1
    content = '\n'.join(new_lines)
    
    if content != original:
        path.write_text(content, encoding='utf-8')
        return True
    return False

def fix_docs(path: Path) -> bool:
    content = path.read_text(encoding='utf-8')
    original = content
    
    # Fix MD040: Code blocks missing language
    content = re.sub(r'^```\n', '```text\n', content, flags=re.MULTILINE)
    
    if content != original:
        path.write_text(content, encoding='utf-8')
        return True
    return False

def main():
    skills_dir = Path('.agents/skills')
    fixed = 0
    
    # Fix .agents/skills/**/*.md
    for md_file in skills_dir.rglob('*.md'):
        if fix_file(md_file):
            fixed += 1
            print(f"Fixed: {md_file}")
    
    # Fix CHANGELOG.md
    changelog = Path('CHANGELOG.md')
    if changelog.exists() and fix_changelog(changelog):
        fixed += 1
        print(f"Fixed: {changelog}")
    
    # Fix docs/*.md
    docs_dir = Path('docs')
    for md_file in docs_dir.glob('*.md'):
        if fix_docs(md_file):
            fixed += 1
            print(f"Fixed: {md_file}")
    
    print(f"\nTotal files fixed: {fixed}")

if __name__ == '__main__':
    main()