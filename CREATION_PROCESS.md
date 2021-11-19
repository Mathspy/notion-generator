At first I started by looking for a good markdown test file to use for testing styling of HTML
output generated from markdown. But all the tests were honestly kinda just bad? So I ended up using
[the GitHub markdown writing guide](https://docs.github.com/en/github/writing-on-github/getting-started-with-writing-and-formatting-on-github/basic-writing-and-formatting-syntax#styling-text)
to figure out all the typical features of markdown and just created [markdown_test.md](./notion_spec/markdown_test.md)
on my own.

After a short break I checked Notion and realized that they don't actually return markdown OR EVEN
HTML back from their API, but the markdown file was not to go to waste! Simply importing it into
Notion proved super helpful as it exposed most of the features they typically support, after some
slight fixing of links and adding of Notion-specific features it was perfect.

Then I got a Notion private integration up, after a bit of fiddling realized I had to invite the
integration to have access to the page, then I was finally able to make requests to the page, you
can see the requests and their responses in [request.sh](./notion_spec/request.sh) and
[responses folder](./notion_spec/responses) representatively
