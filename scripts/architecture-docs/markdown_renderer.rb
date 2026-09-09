# frozen_string_literal: true

require 'cgi'

module ArchitectureDocs
  module MarkdownRenderer
    Heading = Struct.new(:level, :id, :text)
    Document = Struct.new(:html, :headings, :diagram_count)

    module_function

    def render(markdown)
      render_document(markdown).html
    end

    def render_document(markdown)
      Renderer.new(markdown).render
    end

    class Renderer
      def initialize(markdown)
        @lines = visible_lines(markdown)
        @headings = []
        @used_ids = {}
        @diagram_count = 0
      end

      def render
        blocks = []
        index = 0
        while index < @lines.length
          line = @lines[index]
          if line.empty?
            index += 1
          elsif (fence = fence_start(line))
            block, index = render_fence(index, fence)
            blocks << block
          elsif (heading = heading_parts(line))
            blocks << render_heading(heading)
            index += 1
          elsif table_start?(index)
            block, index = render_table(index)
            blocks << block
          elsif list_item(line)
            block, index = render_list(index)
            blocks << block
          elsif quote_line?(line)
            block, index = render_quote(index)
            blocks << block
          else
            block, index = render_paragraph(index)
            blocks << block
          end
        end
        Document.new(blocks.join("\n"), @headings.freeze, @diagram_count)
      end

      private

      def visible_lines(markdown)
        lines = markdown.split("\n", -1).map { |line| line.end_with?("\r") ? line[0...-1] : line }
        in_fence = false
        visible = []
        prose = []
        lines.each do |line|
          if line.match?(/\A\s*```/)
            visible.concat(strip_closed_comments(prose))
            prose = []
            in_fence = !in_fence
            visible << line
          elsif in_fence
            visible << line
          else
            prose << line
          end
        end
        visible.concat(strip_closed_comments(prose))
        visible
      end

      def strip_closed_comments(lines)
        text = lines.join("\n")
        output = +''
        code_ticks = nil
        index = 0
        while index < text.length
          if text[index] == '`'
            run = text[index..-1][/\A`+/].length
            if code_ticks == run
              code_ticks = nil
            elsif code_ticks.nil?
              code_ticks = run
            end
            output << ('`' * run)
            index += run
          elsif code_ticks.nil? && text[index, 4] == '<!--'
            closing = text.index('-->', index + 4)
            unless closing
              output << text[index..-1]
              break
            else
              output << text[index..(closing + 2)].scan("\n").join
              index = closing + 3
            end
          else
            output << text[index]
            index += 1
          end
        end
        output.split("\n", -1)
      end

      def fence_start(line)
        match = line.match(/\A\s*```\s*([^\s`]*)\s*\z/)
        match && match[1].to_s.downcase
      end

      def render_fence(index, language)
        source = []
        cursor = index + 1
        while cursor < @lines.length && !@lines[cursor].match?(/\A\s*```\s*\z/)
          source << @lines[cursor]
          cursor += 1
        end
        cursor += 1 if cursor < @lines.length
        code = source.join("\n")
        code << "\n" unless source.empty?
        if language == 'mermaid'
          @diagram_count += 1
          id = "diagram-#{@diagram_count}"
          html = [
            %(<figure class="diagram" data-diagram-state="pending" id="#{id}">),
            %(<div class="diagram-toolbar"><button type="button" data-diagram-zoom="out">−</button>),
            %(<button type="button" data-diagram-zoom="in">+</button>),
            %(<button type="button" data-diagram-fullscreen>全屏</button></div>),
            %(<div class="mermaid-render" aria-live="polite"></div>),
            %(<pre class="mermaid-source"><code>#{escape_code(code)}</code></pre>),
            %(</figure>)
          ].join
          [html, cursor]
        else
          class_name = language.empty? ? '' : %( class="language-#{escape_attribute(language)}")
          ["<pre><code#{class_name}>#{escape_code(code)}</code></pre>", cursor]
        end
      end

      def heading_parts(line)
        match = line.match(/\A(\#{1,6})[ \t]+(.+?)\s*\z/)
        match && [match[1].length, match[2].sub(/[ \t]+#+[ \t]*\z/, '')]
      end

      def render_heading(parts)
        level, raw_text = parts
        base = slug(raw_text)
        id = unique_id(base)
        plain_text = plain(raw_text)
        @headings << Heading.new(level, id, plain_text)
        %(<h#{level} id="#{escape_attribute(id)}">#{inline(raw_text)}</h#{level}>)
      end

      def slug(text)
        value = plain(text).downcase
        value = value.gsub(/[^\p{Alnum}]+/u, '-').gsub(/\A-+|-+\z/, '')
        value.empty? ? 'section' : value
      end

      def unique_id(base)
        candidate = base
        suffix = 2
        while @used_ids[candidate]
          candidate = "#{base}-#{suffix}"
          suffix += 1
        end
        @used_ids[candidate] = true
        candidate
      end

      def plain(text)
        text.gsub(/`+/, '').gsub(/\*\*/, '').gsub(/\[([^\]]+)\]\([^)]*\)/, '\\1').strip
      end

      def table_start?(index)
        return false unless index + 1 < @lines.length

        header = split_table_row(@lines[index])
        separator = split_table_row(@lines[index + 1])
        return false unless header.length >= 2 && header.length == separator.length

        separator.all? { |cell| cell.strip.match?(/\A:?-{3,}:?\z/) }
      end

      def render_table(index)
        headers = split_table_row(@lines[index])
        cursor = index + 2
        rows = []
        while cursor < @lines.length && @lines[cursor].include?('|')
          cells = split_table_row(@lines[cursor])
          break unless cells.length == headers.length

          rows << cells
          cursor += 1
        end
        head = headers.map { |cell| "<th>#{inline(cell.strip)}</th>" }.join
        body = rows.map do |row|
          '<tr>' + row.map { |cell| "<td>#{inline(cell.strip)}</td>" }.join + '</tr>'
        end.join
        ["<table><thead><tr>#{head}</tr></thead><tbody>#{body}</tbody></table>", cursor]
      end

      def split_table_row(line)
        value = line.strip
        value = value[1..-1] if value.start_with?('|')
        value = value[0...-1] if value.end_with?('|') && !value.end_with?('\\|')
        cells = []
        cell = +''
        code_ticks = nil
        index = 0
        while index < value.length
          character = value[index]
          if character == '\\'
            following = value[index + 1]
            if following == '|' || following == '\\'
              cell << following
              index += 1
            else
              cell << character
            end
          elsif character == '`'
            run = value[index..-1][/\A`+/].length
            if code_ticks == run
              code_ticks = nil
            elsif code_ticks.nil?
              code_ticks = run
            end
            cell << ('`' * run)
            index += run - 1
          elsif character == '|' && code_ticks.nil?
            cells << cell
            cell = +''
          else
            cell << character
          end
          index += 1
        end
        cells << cell
        cells
      end

      def list_item(line)
        if (match = line.match(/\A\s*[-+*][ \t]+(.+)\z/))
          [:ul, match[1]]
        elsif (match = line.match(/\A\s*\d+[.)][ \t]+(.+)\z/))
          [:ol, match[1]]
        end
      end

      def render_list(index)
        type, = list_item(@lines[index])
        items = []
        cursor = index
        while cursor < @lines.length
          item = list_item(@lines[cursor])
          break unless item && item[0] == type

          items << "<li>#{inline(item[1])}</li>"
          cursor += 1
        end
        ["<#{type}>#{items.join}</#{type}>", cursor]
      end

      def quote_line?(line)
        line.match?(/\A\s*>/)
      end

      def render_quote(index)
        lines = []
        cursor = index
        while cursor < @lines.length && quote_line?(@lines[cursor])
          lines << @lines[cursor].sub(/\A\s*>[ \t]?/, '')
          cursor += 1
        end
        ["<blockquote>#{inline(lines.join("\n"))}</blockquote>", cursor]
      end

      def render_paragraph(index)
        lines = []
        cursor = index
        while cursor < @lines.length && !@lines[cursor].empty? && !block_start?(cursor)
          lines << @lines[cursor]
          cursor += 1
        end
        ["<p>#{inline(lines.join("\n"))}</p>", cursor]
      end

      def block_start?(index)
        line = @lines[index]
        fence_start(line) || heading_parts(line) || table_start?(index) || list_item(line) || quote_line?(line)
      end

      def inline(text)
        output = +''
        plain_buffer = +''
        index = 0
        while index < text.length
          if text[index] == '`'
            run = text[index..-1][/\A`+/].length
            closing = text.index('`' * run, index + run)
            if closing
              output << render_markup(plain_buffer)
              plain_buffer = +''
              code = text[(index + run)...closing]
              output << "<code>#{escape_code(code)}</code>"
              index = closing + run
              next
            end
          end
          plain_buffer << text[index]
          index += 1
        end
        output << render_markup(plain_buffer)
        output
      end

      def render_markup(text)
        output = +''
        buffer = +''
        index = 0
        flush = lambda do
          output << escape_text(buffer)
          buffer = +''
        end
        while index < text.length
          if text[index, 2] == '**'
            closing = text.index('**', index + 2)
            if closing
              flush.call
              output << "<strong>#{escape_text(text[(index + 2)...closing])}</strong>"
              index = closing + 2
              next
            end
          elsif text[index] == '[' && (index.zero? || text[index - 1] != '!')
            label_end = text.index('](', index + 1)
            destination_end = label_end && text.index(')', label_end + 2)
            if destination_end
              destination = text[(label_end + 2)...destination_end]
              if safe_link?(destination)
                flush.call
                label = text[(index + 1)...label_end]
                output << %(<a href="#{escape_attribute(destination)}">#{escape_text(label)}</a>)
                index = destination_end + 1
                next
              end
            end
          end
          buffer << text[index]
          index += 1
        end
        flush.call
        output
      end

      def safe_link?(destination)
        return false if destination.empty? || destination.match?(/[\u0000-\u0020]/)

        normalized = CGI.unescapeHTML(destination).gsub(/[\u0000-\u0020]/, '').downcase
        return true if normalized.start_with?('#')
        return true if normalized.start_with?('https://')
        return false if normalized.start_with?('//')
        return false if normalized.match?(/\A[a-z][a-z0-9+.-]*:/)

        true
      end

      def escape_text(text)
        output = +''
        cursor = 0
        text.to_enum(:scan, /&#(?:\d+|x[0-9a-f]+);/i).each do
          match = Regexp.last_match
          output << CGI.escapeHTML(text[cursor...match.begin(0)])
          output << match[0]
          cursor = match.end(0)
        end
        output << CGI.escapeHTML(text[cursor..-1].to_s)
        output
      end

      def escape_attribute(text)
        CGI.escapeHTML(text)
      end

      def escape_code(text)
        CGI.escapeHTML(text)
      end
    end
  end
end
