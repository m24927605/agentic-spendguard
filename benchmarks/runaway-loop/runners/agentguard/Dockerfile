FROM node:20-slim

WORKDIR /app
COPY package.json .
RUN npm install --omit=dev

COPY runner.js .

CMD ["node", "runner.js"]
