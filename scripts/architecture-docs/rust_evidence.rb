# frozen_string_literal: true

require 'digest'

module ArchitectureDocs
  # A byte-preserving lexical locator, not a Rust type checker. Masked bytes retain
  # line endings and offsets; hashes always cover the original complete lines.
  module RustEvidence
    class Invalid < StandardError; end
    module_function

    def mask(source)
      bytes = source.b
      result = bytes.dup
      index = 0
      while index < bytes.bytesize
        finish = nil
        if bytes.byteslice(index, 2) == '//'
          finish = bytes.index("\n", index) || bytes.bytesize
        elsif bytes.byteslice(index, 2) == '/*'
          depth = 1
          cursor = index + 2
          while depth > 0 && cursor < bytes.bytesize
            pair = bytes.byteslice(cursor, 2)
            if pair == '/*'
              depth += 1
              cursor += 2
            elsif pair == '*/'
              depth -= 1
              cursor += 2
            else
              cursor += 1
            end
          end
          raise Invalid, 'rust_lexical_invalid unterminated_comment' unless depth.zero?
          finish = cursor
        elsif bytes.getbyte(index) == 114 && (raw = /\Ar(\#*)"/.match(bytes.byteslice(index, bytes.bytesize - index)))
          ending = '"' + raw[1]
          closing = bytes.index(ending, index + raw[0].bytesize)
          raise Invalid, 'rust_lexical_invalid unterminated_raw_string' unless closing
          finish = closing + ending.bytesize
        elsif bytes.getbyte(index) == 34
          cursor = index + 1
          while cursor < bytes.bytesize && bytes.getbyte(cursor) != 34
            cursor += bytes.getbyte(cursor) == 92 ? 2 : 1
          end
          raise Invalid, 'rust_lexical_invalid unterminated_string' if cursor >= bytes.bytesize
          finish = cursor + 1
        elsif bytes.getbyte(index) == 39
          # Lifetimes have no closing quote; a character literal has one scalar
          # value or one Rust escape. Match Unicode scalars by their UTF-8 bytes.
          char = /\A'(?:\\(?:u\{[0-9a-fA-F_]+\}|x[0-9a-fA-F]{2}|[^\r\n])|[\xC0-\xF7][\x80-\xBF]{1,3}|[^'\\\r\n\x80-\xFF])'/n.match(bytes.byteslice(index, bytes.bytesize - index))
          finish = index + char[0].bytesize if char
        end
        if finish
          (index...finish).each { |offset| result.setbyte(offset, 32) unless [10, 13].include?(bytes.getbyte(offset)) }
          index = finish
        else
          index += 1
        end
      end
      result
    end

    def locate(source, symbol, kind)
      keyword = { 'rust_fn' => 'fn', 'rust_enum' => 'enum', 'rust_impl' => 'impl', 'rust_mod' => 'mod' }[kind]
      raise Invalid, "evidence_kind_invalid kind=#{kind}" unless keyword
      if kind != 'rust_impl' && !identifier?(symbol)
        raise Invalid, "symbol_identifier_invalid symbol=#{symbol}"
      end
      masked = mask(source)
      matches = []
      if kind == 'rust_impl'
        # Rust items may follow another item or an opening module brace inline.
        masked.to_enum(:scan, /\bimpl\b/n).each do
          declaration = Regexp.last_match
          offset = masked.index('impl', declaration.begin(0))
          next unless impl_item_boundary?(masked, offset)
          opening = body_opening(masked, offset)
          next unless opening
          header = masked.byteslice(offset + 4, opening - offset - 4).split.join(' ')
          matches << offset if header == symbol.b
        end
      else
        # Extract whole declaration identifiers before comparison. User-supplied
        # signature fragments must never select one of several same-named items.
        pattern = /\b#{keyword}\s+((?:r#)?[A-Za-z_\x80-\xff][A-Za-z0-9_\x80-\xff]*)/n
        masked.to_enum(:scan, pattern).each do
          declaration = Regexp.last_match
          matches << declaration.begin(0) if declaration[1] == symbol.b && identifier?(declaration[1])
        end
      end
      raise Invalid, "symbol_missing symbol=#{symbol}" if matches.empty?
      raise Invalid, "symbol_ambiguous symbol=#{symbol}" unless matches.length == 1
      offset = matches.first
      opening = body_opening(masked, offset)
      raise Invalid, "symbol_body_missing symbol=#{symbol}" unless opening
      depth = 1
      cursor = opening + 1
      while depth > 0 && cursor < masked.bytesize
        depth += 1 if masked.getbyte(cursor) == 123
        depth -= 1 if masked.getbyte(cursor) == 125
        cursor += 1
      end
      raise Invalid, "symbol_body_unclosed symbol=#{symbol}" unless depth.zero?
      line_start = masked.rindex("\n", [offset - 1, 0].max)
      line_start = line_start ? line_start + 1 : 0
      line_end = masked.index("\n", cursor)
      line_end = line_end ? line_end + 1 : masked.bytesize
      {
        'symbol_sha256' => Digest::SHA256.hexdigest(source.b.byteslice(line_start, line_end - line_start)),
        'start_line' => masked.byteslice(0, line_start).count("\n") + 1,
        'end_line' => masked.byteslice(0, cursor).count("\n") + 1,
        'body' => masked.byteslice(opening + 1, cursor - opening - 2)
      }
    end

    def identifier?(symbol)
      return false unless symbol.is_a?(String)

      identifier = symbol.dup.force_encoding(Encoding::UTF_8)
      identifier.valid_encoding? && identifier.match?(/\A(?:r#)?[_\p{XID_Start}][\p{XID_Continue}]*\z/)
    end

    def impl_item_boundary?(masked, offset)
      # A return/type-position `impl Trait` or `r#impl` is not an impl item.
      # Work only on masked bytes so comment/string punctuation cannot supply
      # a boundary. Outer attributes may contain nested bracket token trees.
      prefix = masked.byteslice(0, offset).rstrip
      prefix = prefix.sub(/\bunsafe\z/n, '').rstrip
      while prefix.end_with?(']')
        depth = 1
        cursor = prefix.bytesize - 2
        while cursor >= 0 && depth > 0
          depth += 1 if prefix.getbyte(cursor) == 93
          depth -= 1 if prefix.getbyte(cursor) == 91
          cursor -= 1
        end
        return false unless depth.zero?

        # Rust permits whitespace/comments between attribute punctuation.
        # Still require '#', not an arbitrary preceding array/index expression.
        marker = prefix.byteslice(0, cursor + 1).rstrip
        marker = marker.byteslice(0, marker.bytesize - 1).rstrip if marker.end_with?('!')
        return false unless marker.end_with?('#')

        prefix = marker.byteslice(0, marker.bytesize - 1).rstrip
      end
      prefix.empty? || [59, 123, 125].include?(prefix.getbyte(prefix.bytesize - 1))
    end

    def body_opening(masked, offset)
      # Braces and semicolons inside signature arrays/const expressions are not
      # the function body or a declaration terminator.
      opening = nil
      parens = 0
      brackets = 0
      angles = 0
      const_braces = 0
      signature = offset
      while signature < masked.bytesize
        byte = masked.getbyte(signature)
        parens += 1 if byte == 40
        parens -= 1 if byte == 41
        brackets += 1 if byte == 91
        brackets -= 1 if byte == 93
        if parens.zero? && brackets.zero? && const_braces.zero?
          angles += 1 if byte == 60
          angles -= 1 if byte == 62 && angles > 0 && masked.getbyte(signature - 1) != 45
        end
        const_braces += 1 if byte == 123 && angles > 0
        const_braces -= 1 if byte == 125 && const_braces > 0
        if parens.zero? && brackets.zero? && angles.zero? && const_braces.zero?
          if byte == 123
            opening = signature
            break
          end
          break if byte == 59
        end
        signature += 1
      end
      opening
    end

    def enum_variants(item)
      # PushKind is a fieldless enum; unsupported payloads fail closed.
      body = item.fetch('body')
      raise Invalid, 'enum_shape_unsupported' unless body.match?(/\A[\s,A-Za-z0-9_]*\z/n)
      body.split(',').map(&:strip).reject(&:empty?)
    end
  end
end
